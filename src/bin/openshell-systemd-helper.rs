// SPDX-FileCopyrightText: Copyright (c) 2026 Daniel Rahn
// SPDX-License-Identifier: Apache-2.0

//! The privileged helper of the systemd compute driver.
//!
//! The driver runs as the gateway's user and cannot start system units.
//! This helper runs as root and does that for it, and nothing else: it
//! writes a sandbox's launch files to a directory of its own and starts,
//! stops and reads the state of that sandbox's instances of three unit
//! templates shipped with it. The only thing of a request that reaches a
//! path or a unit name is the sandbox ID, a canonical UUID. It serves one
//! user, checked from the socket's peer credentials.

use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use clap::Parser;
use miette::{IntoDiagnostic, Result, miette};
use openshell_driver_systemd::config::DEFAULT_HELPER_SOCKET;
use openshell_driver_systemd::envfile;
use openshell_driver_systemd::fence::{Fence, Mode, Outcome};
use openshell_driver_systemd::protocol::{
    MAX_MESSAGE_BYTES, Provision, Request, Response, SandboxRecord, UnitState, valid_sandbox_id,
};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tracing::{info, warn};

/// Where the unit templates expect each sandbox's directory.
const DEFAULT_STATE_DIR: &str = "/var/lib/openshell/systemd";

/// Mount points the unit templates bind a sandbox's files to. They are the
/// paths the stock supervisor and `openshell-sandbox` use in a container.
const MOUNT_POINT_DIRECTORIES: &[&str] = &[
    "/.openshell/supervisor",
    "/.openshell/channel",
    "/sandbox",
    "/etc/openshell/auth",
    "/etc/openshell/tls/client",
    "/etc/userdb",
];
const MOUNT_POINT_FILES: &[&str] = &[
    "/etc/openshell/auth/sandbox.jwt",
    "/etc/openshell/tls/client/ca.crt",
];

/// The workload's resolver is `openshell-sandbox`'s policy DNS.
/// The programs the helper runs, by their paths: not whatever a `PATH`
/// finds first.
const SYSTEMCTL: &str = "/usr/bin/systemctl";
const CHCON: &str = "/usr/bin/chcon";
const STAT: &str = "/usr/bin/stat";

/// How long a client may take to send its request. Requests are served
/// one at a time, so a client that sends nothing would hold up every
/// other.
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

const RESOLV_CONF: &str = "nameserver 127.0.0.53\noptions edns0 trust-ad\n";

#[derive(Parser)]
#[command(name = "openshell-systemd-helper")]
#[command(version = openshell_driver_systemd::VERSION)]
struct Args {
    /// The socket the driver reaches the helper on.
    #[arg(long, env = "OPENSHELL_SYSTEMD_HELPER_SOCKET", default_value = DEFAULT_HELPER_SOCKET)]
    socket: PathBuf,

    /// The only user served: the one the gateway and its driver run as.
    #[arg(long, env = "OPENSHELL_SYSTEMD_HELPER_ALLOW_USER")]
    allow_user: String,

    /// The user IDs sandboxes are given, each sandbox one of its own, as
    /// `FIRST-LAST`. The group has the same number. An ID that an account
    /// of the host has is left out.
    #[arg(
        long,
        env = "OPENSHELL_SYSTEMD_UID_RANGE",
        default_value = "70000-79999"
    )]
    uid_range: UidRange,

    /// Where sandboxes' files are kept. The unit templates name the default.
    #[arg(long, default_value = DEFAULT_STATE_DIR)]
    state_dir: PathBuf,

    #[arg(long, env = "OPENSHELL_LOG_LEVEL", default_value = "info")]
    log_level: String,

    /// The Landlock fence the helper puts around itself before it serves
    /// a request: `required` fails on a kernel that cannot hold it to its
    /// files, `best-effort` goes on with what the kernel offers, `off`
    /// asks for none.
    #[arg(
        long,
        env = "OPENSHELL_SYSTEMD_HELPER_LANDLOCK",
        value_enum,
        default_value = "required"
    )]
    landlock: Mode,

    /// Enter the fence, try what it must refuse and what it must allow,
    /// print each result and exit: 0 if all are as they must be.
    #[arg(long)]
    self_test: bool,
}

/// Where a sandbox's units get their drop-ins.
const UNIT_DIR: &str = "/run/systemd/system";

/// Where systemd looks for user and group records besides the files of
/// `/etc`: a sandbox's user gets its name here, for as long as the host
/// runs. Nothing is written to `/etc/passwd`.
const USERDB_DIR: &str = "/run/userdb";

/// The user IDs sandboxes are given.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct UidRange {
    first: u32,
    last: u32,
}

impl UidRange {
    /// What a range must leave out: the system's accounts below, and the
    /// IDs systemd and the kernel keep for themselves (home directories
    /// of systemd-homed, dynamic users, "nobody") in between.
    const FIRST_ALLOWED: u32 = 1000;
    const RESERVED: std::ops::RangeInclusive<u32> = 60001..=65535;
    const LAST_ALLOWED: u32 = 0x7fff_fffe;
}

impl std::str::FromStr for UidRange {
    type Err = String;

    fn from_str(text: &str) -> std::result::Result<Self, String> {
        let number = |text: &str| {
            text.trim()
                .parse::<u32>()
                .map_err(|_| format!("{text:?} is not a user ID"))
        };
        let (first, last) = text
            .split_once('-')
            .ok_or_else(|| format!("{text:?} is not a range FIRST-LAST"))?;
        let (first, last) = (number(first)?, number(last)?);
        if first > last {
            return Err(format!("{first} is after {last}"));
        }
        if first < Self::FIRST_ALLOWED || last > Self::LAST_ALLOWED {
            return Err(format!(
                "user IDs for sandboxes are from {} to {}",
                Self::FIRST_ALLOWED,
                Self::LAST_ALLOWED
            ));
        }
        if first <= *Self::RESERVED.end() && last >= *Self::RESERVED.start() {
            return Err(format!(
                "the range must leave out {} to {}, which the system keeps for itself",
                Self::RESERVED.start(),
                Self::RESERVED.end()
            ));
        }
        Ok(Self { first, last })
    }
}

/// Whether an account or a group of the host has this number: such an ID
/// is never a sandbox's. A lookup that fails counts as taken.
#[cfg(not(test))]
fn host_account(id: u32) -> bool {
    !matches!(
        nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(id)),
        Ok(None)
    ) || !matches!(
        nix::unistd::Group::from_gid(nix::unistd::Gid::from_raw(id)),
        Ok(None)
    )
}

/// The tests' IDs are whatever the test runs as.
#[cfg(test)]
fn host_account(_id: u32) -> bool {
    false
}

/// A sandbox's group: of the same number as its user.
#[cfg(not(test))]
const fn sandbox_gid(uid: u32) -> u32 {
    uid
}

/// A test cannot give a file to a group it is not in.
#[cfg(test)]
fn sandbox_gid(_uid: u32) -> u32 {
    nix::unistd::getegid().as_raw()
}

/// Whether a number read from a sandbox's directory can be its user:
/// never root or a system account, whatever the file says.
#[cfg(not(test))]
const fn sandbox_uid(uid: u32) -> bool {
    uid >= UidRange::FIRST_ALLOWED
}

/// A test's sandboxes are the user the test runs as, root in a package
/// build.
#[cfg(test)]
const fn sandbox_uid(_uid: u32) -> bool {
    true
}

/// The name a sandbox's user and group have on the host.
fn sandbox_user_name(uid: u32) -> String {
    format!("osb-{uid}")
}

/// The records that give a sandbox's user and group their name, as the
/// files systemd reads from a `userdb` directory: a record under the
/// name, and a link to it under the number. Each entry is a file's name
/// and either its content or the name it links to.
fn userdb_records(uid: u32) -> [(String, std::result::Result<String, String>); 4] {
    let name = sandbox_user_name(uid);
    let gid = sandbox_gid(uid);
    let user_record = serde_json::json!({
        "userName": name,
        "uid": uid,
        "gid": gid,
        "realName": "OpenShell sandbox",
        "homeDirectory": "/sandbox",
        "shell": "/usr/sbin/nologin",
    });
    let group_record = serde_json::json!({ "groupName": name, "gid": gid });
    let user_file = format!("{name}.user");
    let group_file = format!("{name}.group");
    [
        (user_file.clone(), Ok(user_record.to_string())),
        (format!("{uid}.user"), Err(user_file)),
        (group_file.clone(), Ok(group_record.to_string())),
        (format!("{gid}.group"), Err(group_file)),
    ]
}

/// Writes a sandbox's records into a `userdb` directory. A file that is
/// there is replaced, by a new file under its name.
fn write_userdb(dir: &Path, uid: u32, owner: u32, group: u32) -> std::io::Result<()> {
    for (file, entry) in userdb_records(uid) {
        let path = dir.join(file);
        remove_tree(&path)?;
        match entry {
            Ok(content) => create_file(&path, content.as_bytes(), 0o644, owner, group)?,
            Err(target) => std::os::unix::fs::symlink(target, &path)?,
        }
    }
    Ok(())
}

fn remove_userdb(dir: &Path, uid: u32) {
    for (file, _) in userdb_records(uid) {
        let _ = remove_tree(&dir.join(file));
    }
}

/// What the helper needs once it serves, and what the programs it runs
/// need (`systemctl`, `restorecon`, `chcon`, `stat`): everything else of
/// the host is out of its reach, as root too.
fn fence(state_dir: &Path, socket: &Path) -> Fence {
    let paths = |paths: &[&str]| paths.iter().map(PathBuf::from).collect::<Vec<_>>();
    let mut read_write = vec![
        state_dir.to_path_buf(),
        PathBuf::from(UNIT_DIR),
        PathBuf::from(USERDB_DIR),
    ];
    read_write.extend(socket.parent().map(Path::to_path_buf));
    // /dev/null alone of the devices, and the files through which a label
    // is checked.
    read_write.extend(paths(&["/dev/null", "/sys/fs/selinux"]));
    Fence {
        read_execute: paths(&["/usr"]),
        // The mount points are read for their labels; the journal for a
        // failed unit's last lines.
        read: paths(&[
            "/etc",
            "/dev/urandom",
            "/proc",
            "/sys",
            "/run",
            "/var/log/journal",
            "/.openshell",
            "/sandbox",
        ]),
        read_write,
        no_tcp: true,
    }
}

/// Tries, inside the fence, what it must refuse and what it must allow.
fn self_test(state_dir: &Path, outcome: Outcome) -> bool {
    let denied = |error: &std::io::Error| error.kind() == std::io::ErrorKind::PermissionDenied;
    let probe = state_dir.join(".self-test");
    let etc = Path::new("/etc/openshell-systemd-helper-self-test");
    let checks = [
        (
            "the kernel holds the helper to its files",
            outcome != Outcome::None,
        ),
        (
            "its state directory is written",
            std::fs::write(&probe, "x").is_ok() && std::fs::remove_file(&probe).is_ok(),
        ),
        (
            "a program in /usr is read",
            std::fs::read("/usr/bin/stat").is_ok(),
        ),
        ("/etc is read", std::fs::read("/etc/passwd").is_ok()),
        (
            "nothing is created in /etc",
            std::fs::write(etc, "x").is_err_and(|error| denied(&error)),
        ),
        (
            "/root is not read",
            std::fs::read_dir("/root").is_err_and(|error| denied(&error)),
        ),
        (
            "/home is not read",
            std::fs::read_dir("/home").is_err_and(|error| denied(&error)),
        ),
        (
            "/var/lib is not read beyond its own",
            std::fs::read_dir("/var/lib").is_err_and(|error| denied(&error)),
        ),
        (
            "no TCP port is taken",
            std::net::TcpListener::bind("127.0.0.1:0").is_err_and(|error| denied(&error)),
        ),
    ];
    let mut all = true;
    for (what, passed) in checks {
        println!("{} {what}", if passed { "ok  " } else { "FAIL" });
        all &= passed;
    }
    all
}

/// A sandbox's pair of `SELinux` categories, two of `c0` to `c1023`. Its
/// supervisor, its workload and its files carry them, so that the policy
/// refuses one sandbox what is another's even where the namespaces and
/// file modes would not: every sandbox runs in the same domain and as the
/// same user.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
struct Categories(u16, u16);

impl Categories {
    const COUNT: u16 = 1024;

    fn level(self) -> String {
        format!("s0:c{},c{}", self.0, self.1)
    }
}

/// What the helper keeps of a sandbox besides its launch files.
#[derive(Debug, Deserialize, Serialize)]
struct Metadata {
    name: String,
    workspace: String,
    /// Kept for the sandbox's life: its workspace carries them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    categories: Option<Categories>,
}

/// The units that run in a domain of the policy module, and the domain.
const CONFINED_UNITS: &[(&str, &str)] = &[
    ("supervisor", "openshell_supervisor_t"),
    ("workload", "openshell_sandbox_t"),
];

struct Helper {
    state_dir: PathBuf,
    /// Where a sandbox's units get their drop-ins: systemd's runtime
    /// directory for units.
    unit_dir: PathBuf,
    /// Whether the policy module is in use, and sandboxes get categories.
    categories: bool,
    /// The owner of what only the helper and systemd read: root.
    owner_uid: u32,
    owner_gid: u32,
    /// The user IDs sandboxes are given.
    uid_range: UidRange,
    /// Where the sandboxes' users get their names: systemd's runtime
    /// directory for user records.
    userdb_dir: PathBuf,
}

fn unit(kind: &str, sandbox_id: &str) -> String {
    format!("openshell-sandbox-{kind}@{sandbox_id}.service")
}

/// Creates a directory with exactly `mode`, owned by `uid:gid`.
///
/// Owner and mode are set on the opened directory, not by its path: some
/// of these directories are inside one that the workloads' user owns, and
/// a name there could be made a symbolic link between two steps.
fn create_directory(path: &Path, mode: u32, uid: u32, gid: u32) -> std::io::Result<()> {
    std::fs::DirBuilder::new().mode(mode).create(path)?;
    let directory = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_DIRECTORY | nix::libc::O_NOFOLLOW)
        .open(path)?;
    // The mode given at creation is subject to the umask.
    directory.set_permissions(std::fs::Permissions::from_mode(mode))?;
    std::os::unix::fs::fchown(&directory, Some(uid), Some(gid))
}

/// Creates a file that must not exist yet, with exactly `mode`, owned by
/// `uid:gid`.
fn create_file(path: &Path, content: &[u8], mode: u32, uid: u32, gid: u32) -> std::io::Result<()> {
    use std::io::Write as _;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)?;
    file.write_all(content)?;
    file.set_permissions(std::fs::Permissions::from_mode(mode))?;
    // On the file that was created, whatever its name points to by now.
    std::os::unix::fs::fchown(&file, Some(uid), Some(gid))
}

/// Reads a file that must be a regular file, not a link to one.
fn read_plain_file(path: &Path) -> std::io::Result<String> {
    use std::io::Read as _;

    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other("not a regular file"));
    }
    let mut content = String::new();
    file.read_to_string(&mut content)?;
    Ok(content)
}

/// The environment a supervisor may be given: `OpenShell`'s own settings.
/// Not this driver's (`OPENSHELL_SYSTEMD_…`), which belong to the
/// administrator, and nothing that changes how the unit's programs are
/// loaded (`LD_PRELOAD`, `PATH`).
fn supervisor_variable(name: &str) -> bool {
    name.starts_with("OPENSHELL_") && !name.starts_with("OPENSHELL_SYSTEMD_")
}

fn remove_tree(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
        Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
    }
}

/// Gives files the `SELinux` labels the loaded policy has for their paths.
/// A new file takes its directory's label, and the units' domains
/// (`selinux/openshell_systemd.te`) tell a sandbox's files apart by label.
/// `restorecon` does nothing where `SELinux` is off; a host without it has
/// no labels to set.
fn relabel(arguments: &[&std::ffi::OsStr]) -> std::result::Result<(), String> {
    const RESTORECON: &str = "/usr/sbin/restorecon";
    if !Path::new(RESTORECON).exists() {
        return Ok(());
    }
    match std::process::Command::new(RESTORECON)
        .args(arguments)
        .output()
    {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(format!(
            "restorecon: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(error) => Err(format!("restorecon: {error}")),
    }
}

/// Whether the policy module labels the programs: then the units run in
/// its domains, and a sandbox gets categories. `stat` prints a file's
/// `SELinux` context with `%C`.
///
/// Where `SELinux` is on and the label cannot be read, that is an error:
/// to go on would be to run every sandbox without categories.
fn policy_module_in_use() -> std::result::Result<bool, String> {
    if !Path::new("/sys/fs/selinux/enforce").exists() {
        return Ok(false);
    }
    let output = std::process::Command::new(STAT)
        .args(["-c", "%C", "/usr/libexec/openshell/openshell-sandbox"])
        .output()
        .map_err(|error| format!("{STAT}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{STAT}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).contains(":openshell_sandbox_exec_t:"))
}

/// Gives files a level, as `chcon -l` does.
fn set_level(level: &str, recursive: bool, paths: &[PathBuf]) -> std::result::Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut command = std::process::Command::new(CHCON);
    if recursive {
        command.arg("-R");
    }
    match command.args(["-l", level]).args(paths).output() {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(format!(
            "chcon: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(error) => Err(format!("chcon: {error}")),
    }
}

/// The two units' states as systemd names them, as one state.
fn unit_state(supervisor: &str, workload: &str) -> UnitState {
    let transitioning = |state: &str| matches!(state, "activating" | "deactivating" | "reloading");
    if supervisor == "active" && workload == "active" {
        UnitState::Active
    } else if supervisor == "failed" || workload == "failed" {
        UnitState::Failed
    } else if transitioning(supervisor) || transitioning(workload) {
        UnitState::Transitioning
    } else if supervisor == "active" || workload == "active" {
        // One without the other is not a sandbox.
        UnitState::Failed
    } else {
        UnitState::Inactive
    }
}

impl Helper {
    fn sandbox_dir(&self, sandbox_id: &str) -> PathBuf {
        self.state_dir.join(sandbox_id)
    }

    fn metadata(&self, sandbox_id: &str) -> Option<Metadata> {
        let encoded = std::fs::read(self.sandbox_dir(sandbox_id).join("meta.json")).ok()?;
        serde_json::from_slice(&encoded).ok()
    }

    /// A sandbox's user, once one was reserved for it. Kept in a file of
    /// its own, beside the metadata of a launch: the user is there before
    /// the first launch, and for the sandbox's life.
    fn user_of(&self, sandbox_id: &str) -> Option<u32> {
        read_plain_file(&self.sandbox_dir(sandbox_id).join("uid"))
            .ok()?
            .trim()
            .parse()
            .ok()
            .filter(|uid| sandbox_uid(*uid))
    }

    /// The users of every sandbox.
    fn users_in_use(&self) -> std::collections::HashSet<u32> {
        std::fs::read_dir(&self.state_dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter_map(|sandbox_id| self.user_of(&sandbox_id))
            .collect()
    }

    /// A sandbox's user: the one it has, or the first of the range that
    /// no sandbox and no account of the host has. The sandbox's directory
    /// is made for it if it is not there yet; without launch files it is
    /// not yet a sandbox to any other request.
    fn reserve(&self, sandbox_id: &str) -> std::result::Result<u32, String> {
        if let Some(uid) = self.user_of(sandbox_id) {
            return Ok(uid);
        }
        let in_use = self.users_in_use();
        let uid = (self.uid_range.first..=self.uid_range.last)
            .find(|uid| !in_use.contains(uid) && !host_account(*uid))
            .ok_or_else(|| {
                format!(
                    "no free user ID from {} to {}",
                    self.uid_range.first, self.uid_range.last
                )
            })?;
        let dir = self.sandbox_dir(sandbox_id);
        let describe = |what: &str, error: std::io::Error| format!("{what}: {error}");
        if !dir.exists() {
            create_directory(&dir, 0o711, self.owner_uid, self.owner_gid)
                .map_err(|e| describe("sandbox directory", e))?;
        }
        remove_tree(&dir.join("uid.new")).map_err(|e| describe("uid", e))?;
        create_file(
            &dir.join("uid.new"),
            uid.to_string().as_bytes(),
            0o600,
            self.owner_uid,
            self.owner_gid,
        )
        .and_then(|()| std::fs::rename(dir.join("uid.new"), dir.join("uid")))
        .map_err(|e| describe("uid", e))?;
        Ok(uid)
    }

    /// The categories of every sandbox but one.
    fn categories_in_use(&self, except: &str) -> std::collections::HashSet<Categories> {
        std::fs::read_dir(&self.state_dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|sandbox_id| sandbox_id != except)
            .filter_map(|sandbox_id| self.metadata(&sandbox_id)?.categories)
            .collect()
    }

    /// A sandbox's categories: those it has, or a pair no other sandbox has.
    fn categories_of(&self, sandbox_id: &str) -> std::result::Result<Categories, String> {
        if let Some(categories) = self
            .metadata(sandbox_id)
            .and_then(|metadata| metadata.categories)
        {
            return Ok(categories);
        }
        let in_use = self.categories_in_use(sandbox_id);
        for _ in 0..1000 {
            let random = openshell_crypto::random_bytes::<4>()
                .map_err(|error| format!("choose categories: {error}"))?;
            let first = u16::from_le_bytes([random[0], random[1]]) % Categories::COUNT;
            let second = u16::from_le_bytes([random[2], random[3]]) % Categories::COUNT;
            let categories = Categories(first.min(second), first.max(second));
            if first != second && !in_use.contains(&categories) {
                return Ok(categories);
            }
        }
        Err("no free pair of categories".to_string())
    }

    /// Gives a sandbox's files its level. The workspace's own files have it
    /// from the workload that made them; they are gone through only when
    /// the workspace's directory does not have it yet.
    fn label_files(
        &self,
        sandbox_id: &str,
        categories: Categories,
    ) -> std::result::Result<(), String> {
        let dir = self.sandbox_dir(sandbox_id);
        let workspace = dir.join("workspace");
        let level = categories.level();
        let labelled = std::process::Command::new(STAT)
            .args(["-c", "%C"])
            .arg(&workspace)
            .output()
            .is_ok_and(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .ends_with(&level)
            });
        let others: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map_err(|error| format!("read {}: {error}", dir.display()))?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| *path != workspace)
            .collect();
        set_level(&level, false, std::slice::from_ref(&dir))?;
        set_level(&level, true, &others)?;
        set_level(&level, !labelled, &[workspace])
    }

    fn drop_in(&self, kind: &str, sandbox_id: &str) -> PathBuf {
        self.unit_dir.join(format!("{}.d", unit(kind, sandbox_id)))
    }

    /// Writes the drop-ins that start a sandbox's units as its user and,
    /// where the policy module is in use, at its level. Returns whether
    /// systemd has to read its units again.
    fn write_drop_ins(
        &self,
        sandbox_id: &str,
        uid: u32,
        categories: Option<Categories>,
    ) -> std::result::Result<bool, String> {
        let mut changed = false;
        for (kind, domain) in CONFINED_UNITS {
            let dir = self.drop_in(kind, sandbox_id);
            // The user by number: it needs no entry anywhere to start.
            let mut files = vec![(
                "40-user.conf",
                format!("[Service]\nUser={uid}\nGroup={}\n", sandbox_gid(uid)),
            )];
            if let Some(categories) = categories {
                files.push((
                    "50-selinux.conf",
                    format!(
                        "[Service]\nSELinuxContext=system_u:system_r:{domain}:{}\n",
                        categories.level()
                    ),
                ));
            }
            for (name, content) in files {
                let file = dir.join(name);
                if std::fs::read_to_string(&file).is_ok_and(|current| current == content) {
                    continue;
                }
                std::fs::create_dir_all(&dir)
                    .and_then(|()| std::fs::write(&file, content))
                    .map_err(|error| format!("write {}: {error}", file.display()))?;
                changed = true;
            }
        }
        Ok(changed)
    }

    /// Removes a sandbox's drop-ins. Returns whether there were any.
    fn remove_drop_ins(&self, sandbox_id: &str) -> bool {
        // Every one of them, also after the first that was there.
        let mut any = false;
        for (kind, _) in CONFINED_UNITS {
            any |= std::fs::remove_dir_all(self.drop_in(kind, sandbox_id)).is_ok();
        }
        any
    }

    /// Writes one launch's files. Everything of an earlier launch but the
    /// workspace is removed first; the files are created, never opened, so
    /// nothing that was there can be written through. Returns the
    /// sandbox's categories, where the policy module is in use.
    fn write_launch_files(
        &self,
        provision: &Provision,
    ) -> std::result::Result<Option<Categories>, String> {
        let describe = |what: &str, error: std::io::Error| format!("{what}: {error}");
        if let Some(name) = provision
            .environment
            .keys()
            .find(|name| !supervisor_variable(name))
        {
            return Err(format!(
                "environment variable {name:?} is not one a supervisor may be given"
            ));
        }
        let environment = envfile::render(&provision.environment).map_err(|e| e.to_string())?;
        // A sandbox that has categories keeps them, whatever this start of
        // the helper found: its files carry them.
        let kept = self
            .metadata(&provision.sandbox_id)
            .and_then(|metadata| metadata.categories);
        let categories = match kept {
            Some(categories) => Some(categories),
            None if self.categories => Some(self.categories_of(&provision.sandbox_id)?),
            None => None,
        };
        let metadata = serde_json::to_vec(&Metadata {
            name: provision.name.clone(),
            workspace: provision.workspace.clone(),
            categories,
        })
        .map_err(|error| error.to_string())?;
        let uid = self
            .user_of(&provision.sandbox_id)
            .ok_or("no user was reserved for the sandbox")?;
        let gid = sandbox_gid(uid);
        let dir = self.sandbox_dir(&provision.sandbox_id);
        let sandbox_token = if provision.sandbox_token.is_empty() {
            read_plain_file(&dir.join("etc/sandbox.jwt"))
                .ok()
                .filter(|token| !token.is_empty())
                .ok_or("no gateway token, and none is kept from an earlier launch")?
        } else {
            provision.sandbox_token.clone()
        };
        if !dir.exists() {
            create_directory(&dir, 0o711, self.owner_uid, self.owner_gid)
                .map_err(|e| describe("sandbox directory", e))?;
        }
        for name in [
            "channel",
            "control",
            "supervisor",
            "etc",
            "supervisor.env",
            "resolv.conf",
            "userdb",
            "meta.json.new",
        ] {
            remove_tree(&dir.join(name)).map_err(|e| describe(name, e))?;
        }

        // The channel: read-only for the supervisor, the sandbox's
        // credentials for the workload alone.
        create_directory(&dir.join("channel"), 0o755, self.owner_uid, self.owner_gid)
            .map_err(|e| describe("channel", e))?;
        let sandbox = dir.join("channel/sandbox");
        create_directory(&sandbox, 0o711, uid, gid).map_err(|e| describe("channel/sandbox", e))?;
        for (name, content) in [
            ("bootstrap.json", &provision.bootstrap),
            ("server.crt", &provision.server_certificate),
            ("server.key", &provision.server_key),
        ] {
            create_file(&sandbox.join(name), content.as_bytes(), 0o600, uid, gid)
                .map_err(|e| describe(name, e))?;
        }
        // The control socket's directory is mounted over this one.
        create_directory(
            &dir.join("channel/control"),
            0o755,
            self.owner_uid,
            self.owner_gid,
        )
        .map_err(|e| describe("channel/control", e))?;
        create_directory(&dir.join("control"), 0o1777, self.owner_uid, self.owner_gid)
            .map_err(|e| describe("control", e))?;

        // The supervisor's credentials, which the workload's unit never
        // mounts.
        let supervisor = dir.join("supervisor");
        create_directory(&supervisor, 0o700, uid, gid).map_err(|e| describe("supervisor", e))?;
        for (name, content) in [
            ("runtime-descriptor.json", &provision.runtime_descriptor),
            ("auth.json", &provision.auth_bundle),
        ] {
            create_file(&supervisor.join(name), content.as_bytes(), 0o600, uid, gid)
                .map_err(|e| describe(name, e))?;
        }
        let etc = dir.join("etc");
        create_directory(&etc, 0o700, uid, gid).map_err(|e| describe("etc", e))?;
        for (name, content) in [
            ("sandbox.jwt", &sandbox_token),
            ("ca.crt", &provision.gateway_ca),
        ] {
            create_file(&etc.join(name), content.as_bytes(), 0o600, uid, gid)
                .map_err(|e| describe(name, e))?;
        }
        // systemd reads the environment file as root.
        create_file(
            &dir.join("supervisor.env"),
            environment.as_bytes(),
            0o600,
            self.owner_uid,
            self.owner_gid,
        )
        .map_err(|e| describe("supervisor.env", e))?;
        create_file(
            &dir.join("resolv.conf"),
            RESOLV_CONF.as_bytes(),
            0o644,
            self.owner_uid,
            self.owner_gid,
        )
        .map_err(|e| describe("resolv.conf", e))?;
        // The name of the sandbox's user, for the workload: its unit
        // mounts this over /etc/userdb. The host's directory of such
        // records names every sandbox's.
        let userdb = dir.join("userdb");
        create_directory(&userdb, 0o755, self.owner_uid, self.owner_gid)
            .and_then(|()| write_userdb(&userdb, uid, self.owner_uid, self.owner_gid))
            .map_err(|e| describe("userdb", e))?;
        let workspace = dir.join("workspace");
        if !workspace.exists() {
            create_directory(&workspace, 0o700, uid, gid).map_err(|e| describe("workspace", e))?;
        }
        // Last, and in one step: until then the metadata of the launch
        // before stands, with the sandbox's categories.
        create_file(
            &dir.join("meta.json.new"),
            &metadata,
            0o600,
            self.owner_uid,
            self.owner_gid,
        )
        .and_then(|()| std::fs::rename(dir.join("meta.json.new"), dir.join("meta.json")))
        .map_err(|e| describe("meta.json", e))?;
        Ok(categories)
    }

    async fn systemctl(arguments: &[&str]) -> std::result::Result<String, String> {
        let output = tokio::process::Command::new(SYSTEMCTL)
            .args(arguments)
            .output()
            .await
            .map_err(|error| format!("run systemctl: {error}"))?;
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if output.status.success() {
            Ok(stdout)
        } else {
            Err(format!(
                "systemctl {}: {} {}",
                arguments.join(" "),
                stdout,
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    }

    /// `systemctl is-active` exits non-zero for every state but active;
    /// the state is on its standard output either way.
    async fn active_state(unit: &str) -> String {
        match tokio::process::Command::new(SYSTEMCTL)
            .args(["is-active", unit])
            .output()
            .await
        {
            Ok(output) => String::from_utf8_lossy(&output.stdout).trim().to_string(),
            Err(_) => "unknown".to_string(),
        }
    }

    /// What systemd says of a sandbox's units, for the error of a start
    /// that failed: their states and results, without their log lines. A
    /// unit's log holds what a workload printed, and the error goes back
    /// to whoever asked for the start; the log is in the journal.
    async fn diagnose(sandbox_id: &str) -> String {
        const LIMIT: usize = 6000;
        let units = [
            unit("netns", sandbox_id),
            unit("supervisor", sandbox_id),
            unit("workload", sandbox_id),
        ];
        let mut arguments = vec!["status", "--no-pager", "--full", "--lines=0"];
        arguments.extend(units.iter().map(String::as_str));
        // `systemctl status` exits non-zero for units that are not active.
        let status = match tokio::process::Command::new(SYSTEMCTL)
            .args(&arguments)
            .output()
            .await
        {
            Ok(output) => String::from_utf8_lossy(&output.stdout).into_owned(),
            Err(error) => format!("systemctl status: {error}"),
        };
        let skip = status.chars().count().saturating_sub(LIMIT);
        status.chars().skip(skip).collect()
    }

    async fn stop_units(sandbox_id: &str) {
        let units = [
            unit("workload", sandbox_id),
            unit("supervisor", sandbox_id),
            unit("netns", sandbox_id),
        ];
        let units: Vec<&str> = units.iter().map(String::as_str).collect();
        let mut stop = vec!["stop"];
        stop.extend(&units);
        if let Err(error) = Self::systemctl(&stop).await {
            warn!(%sandbox_id, %error, "Failed to stop sandbox units");
        }
        let mut reset = vec!["reset-failed"];
        reset.extend(&units);
        // Units that did not fail have nothing to reset.
        let _ = Self::systemctl(&reset).await;
    }

    async fn record(&self, sandbox_id: &str) -> Option<SandboxRecord> {
        let metadata = self.metadata(sandbox_id)?;
        let supervisor = Self::active_state(&unit("supervisor", sandbox_id)).await;
        let workload = Self::active_state(&unit("workload", sandbox_id)).await;
        Some(SandboxRecord {
            sandbox_id: sandbox_id.to_string(),
            name: metadata.name,
            workspace: metadata.workspace,
            state: unit_state(&supervisor, &workload),
            detail: format!("supervisor {supervisor}, workload {workload}"),
        })
    }

    async fn handle(&self, request: Request) -> Response {
        let sandbox_id = match &request {
            Request::Describe | Request::List => None,
            Request::Provision(provision) => Some(provision.sandbox_id.as_str()),
            Request::Reserve { sandbox_id }
            | Request::Start { sandbox_id }
            | Request::Stop { sandbox_id }
            | Request::Delete { sandbox_id }
            | Request::Status { sandbox_id } => Some(sandbox_id.as_str()),
        };
        if sandbox_id.is_some_and(|id| !valid_sandbox_id(id)) {
            return Response::Error {
                message: "the sandbox ID is not a canonical UUID".to_string(),
            };
        }
        match request {
            Request::Describe => Response::Described {
                version: openshell_driver_systemd::VERSION.to_string(),
                first_uid: self.uid_range.first,
                last_uid: self.uid_range.last,
            },
            Request::Reserve { sandbox_id } => match self.reserve(&sandbox_id) {
                Ok(uid) => Response::Reserved {
                    uid,
                    gid: sandbox_gid(uid),
                },
                Err(message) => Response::Error {
                    message: format!("reserve a user for sandbox {sandbox_id}: {message}"),
                },
            },
            Request::Provision(provision) => {
                Self::stop_units(&provision.sandbox_id).await;
                // The workspace's files keep the labels they were made with:
                // only its directory is labelled.
                let dir = self.sandbox_dir(&provision.sandbox_id);
                let workspace = dir.join("workspace");
                let written = self.write_launch_files(&provision).and_then(|categories| {
                    relabel(&[
                        "-R".as_ref(),
                        "-e".as_ref(),
                        workspace.as_os_str(),
                        dir.as_os_str(),
                    ])?;
                    relabel(&[workspace.as_os_str()])?;
                    // restorecon sets the type and leaves the level.
                    categories.map_or(Ok(()), |categories| {
                        self.label_files(&provision.sandbox_id, categories)
                    })
                });
                match written {
                    Ok(()) => Response::Done,
                    Err(message) => Response::Error {
                        message: format!("provision sandbox {}: {message}", provision.sandbox_id),
                    },
                }
            }
            Request::Start { sandbox_id } => {
                let Some(metadata) = self.metadata(&sandbox_id) else {
                    return Response::NotFound;
                };
                let Some(uid) = self.user_of(&sandbox_id) else {
                    return Response::Error {
                        message: format!("sandbox {sandbox_id} has no user"),
                    };
                };
                // The user's name and the drop-ins are gone after a reboot:
                // every start writes what is missing.
                if let Err(error) =
                    write_userdb(&self.userdb_dir, uid, self.owner_uid, self.owner_gid)
                {
                    return Response::Error {
                        message: format!("write the user's name for sandbox {sandbox_id}: {error}"),
                    };
                }
                let reload = match self.write_drop_ins(&sandbox_id, uid, metadata.categories) {
                    Ok(false) => Ok(String::new()),
                    Ok(true) => Self::systemctl(&["daemon-reload"]).await,
                    Err(message) => Err(message),
                };
                if let Err(message) = reload {
                    return Response::Error { message };
                }
                // The workload's unit requires the namespace's and the
                // supervisor's.
                let workload = unit("workload", &sandbox_id);
                match Self::systemctl(&["start", workload.as_str()]).await {
                    Ok(_) => Response::Done,
                    Err(message) => Response::Error {
                        message: format!("{message}\n{}", Self::diagnose(&sandbox_id).await),
                    },
                }
            }
            Request::Stop { sandbox_id } => {
                if self.metadata(&sandbox_id).is_none() {
                    return Response::NotFound;
                }
                Self::stop_units(&sandbox_id).await;
                Response::Done
            }
            Request::Delete { sandbox_id } => {
                let dir = self.sandbox_dir(&sandbox_id);
                if !dir.exists() {
                    return Response::NotFound;
                }
                Self::stop_units(&sandbox_id).await;
                if self.remove_drop_ins(&sandbox_id) {
                    // Only tidying: the units are stopped and their files go.
                    let _ = Self::systemctl(&["daemon-reload"]).await;
                }
                if let Some(uid) = self.user_of(&sandbox_id) {
                    remove_userdb(&self.userdb_dir, uid);
                }
                match std::fs::remove_dir_all(&dir) {
                    Ok(()) => Response::Done,
                    Err(error) => Response::Error {
                        message: format!("remove sandbox {sandbox_id}: {error}"),
                    },
                }
            }
            Request::Status { sandbox_id } => self
                .record(&sandbox_id)
                .await
                .map_or(Response::NotFound, Response::Status),
            Request::List => self.list().await,
        }
    }

    async fn list(&self) -> Response {
        let entries = match std::fs::read_dir(&self.state_dir) {
            Ok(entries) => entries,
            Err(error) => {
                return Response::Error {
                    message: format!("list sandboxes: {error}"),
                };
            }
        };
        let mut sandboxes = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(sandbox_id) = name.to_str() else {
                continue;
            };
            if !valid_sandbox_id(sandbox_id) {
                continue;
            }
            if let Some(record) = self.record(sandbox_id).await {
                sandboxes.push(record);
            }
        }
        sandboxes.sort_by(|a, b| a.sandbox_id.cmp(&b.sandbox_id));
        Response::Listed { sandboxes }
    }

    async fn serve(&self, mut stream: UnixStream) {
        let mut encoded = Vec::new();
        let read = tokio::time::timeout(
            REQUEST_TIMEOUT,
            (&mut stream)
                .take(MAX_MESSAGE_BYTES)
                .read_to_end(&mut encoded),
        )
        .await;
        let response = match read {
            Err(_) => Response::Error {
                message: "the request did not arrive in time".to_string(),
            },
            Ok(Err(error)) => Response::Error {
                message: format!("read the request: {error}"),
            },
            Ok(Ok(_)) => match serde_json::from_slice::<Request>(&encoded) {
                Ok(request) => self.handle(request).await,
                Err(error) => Response::Error {
                    message: format!("unreadable request: {error}"),
                },
            },
        };
        match serde_json::to_vec(&response) {
            Ok(encoded) => {
                if let Err(error) = stream.write_all(&encoded).await {
                    warn!(%error, "Failed to answer the driver");
                }
                let _ = stream.shutdown().await;
            }
            Err(error) => warn!(%error, "Failed to encode an answer"),
        }
    }
}

fn user(name: &str) -> Result<nix::unistd::User> {
    nix::unistd::User::from_name(name)
        .into_diagnostic()?
        .ok_or_else(|| miette!("no user {name}"))
}

/// Binds the helper's socket for `uid` alone.
fn bind(socket: &Path, uid: u32) -> Result<UnixListener> {
    let parent = socket
        .parent()
        .ok_or_else(|| miette!("socket path {} has no parent", socket.display()))?;
    std::fs::create_dir_all(parent).into_diagnostic()?;
    std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o755)).into_diagnostic()?;
    remove_tree(socket).into_diagnostic()?;
    let listener = UnixListener::bind(socket).into_diagnostic()?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600)).into_diagnostic()?;
    std::os::unix::fs::chown(socket, Some(uid), None).into_diagnostic()?;
    Ok(listener)
}

fn main() -> Result<()> {
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_new(&args.log_level)
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();
    if !nix::unistd::geteuid().is_root() {
        return Err(miette!("openshell-systemd-helper must run as root"));
    }
    let allowed = user(&args.allow_user)?.uid.as_raw();
    for directory in MOUNT_POINT_DIRECTORIES {
        std::fs::create_dir_all(directory)
            .map_err(|error| miette!("{}", mount_point_error(directory, &error)))?;
    }
    for file in MOUNT_POINT_FILES {
        if !Path::new(file).exists() {
            create_file(Path::new(file), b"", 0o600, 0, 0).into_diagnostic()?;
        }
    }
    std::fs::create_dir_all(USERDB_DIR).into_diagnostic()?;
    std::fs::create_dir_all(&args.state_dir).into_diagnostic()?;
    std::fs::set_permissions(&args.state_dir, std::fs::Permissions::from_mode(0o711))
        .into_diagnostic()?;
    relabel(&[
        "-R".as_ref(),
        "/.openshell".as_ref(),
        "/sandbox".as_ref(),
        "/etc/openshell".as_ref(),
    ])
    .map_err(|message| miette!("{message}"))?;
    relabel(&[args.state_dir.as_os_str()]).map_err(|message| miette!("{message}"))?;
    let categories = policy_module_in_use().map_err(|message| miette!("{message}"))?;
    if let Some(parent) = args.socket.parent() {
        std::fs::create_dir_all(parent).into_diagnostic()?;
    }
    // The fence, before there is a second thread: it holds the thread that
    // enters it and what that thread starts.
    let fenced = fence(&args.state_dir, &args.socket)
        .enter_as(args.landlock)
        .map_err(|message| miette!("{message}"))?;
    if args.self_test {
        if self_test(&args.state_dir, fenced) {
            return Ok(());
        }
        return Err(miette!("the fence is not as it must be"));
    }
    if fenced == Outcome::None {
        warn!("The helper runs without a Landlock fence");
    }
    let helper = Helper {
        state_dir: args.state_dir,
        unit_dir: PathBuf::from(UNIT_DIR),
        categories,
        owner_uid: 0,
        owner_gid: 0,
        uid_range: args.uid_range,
        userdb_dir: PathBuf::from(USERDB_DIR),
    };
    info!(
        socket = %args.socket.display(),
        allow_user = %args.allow_user,
        first_uid = args.uid_range.first,
        last_uid = args.uid_range.last,
        selinux_categories = categories,
        landlock = ?fenced,
        "Starting systemd helper"
    );
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .into_diagnostic()?
        .block_on(serve(helper, &args.socket, allowed))
}

/// Serves the allowed user's requests until the helper is told to end.
async fn serve(helper: Helper, socket: &Path, allowed: u32) -> Result<()> {
    let listener = bind(socket, allowed)?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .into_diagnostic()?;
    loop {
        tokio::select! {
            _ = terminate.recv() => break,
            accepted = listener.accept() => {
                let (stream, _) = match accepted {
                    Ok(accepted) => accepted,
                    Err(error) => {
                        warn!(%error, "Failed to accept a connection");
                        continue;
                    }
                };
                // Requests are served one at a time: each changes units
                // and files of the same host.
                match stream.peer_cred() {
                    Ok(credentials) if credentials.uid() == allowed => helper.serve(stream).await,
                    Ok(credentials) => warn!(
                        peer_uid = credentials.uid(),
                        "Rejected a helper client that is not the allowed user"
                    ),
                    Err(error) => warn!(%error, "Failed to read a helper client's credentials"),
                }
            }
        }
    }
    let _ = std::fs::remove_file(socket);
    Ok(())
}

/// What a mount point that could not be made means. On a transactional
/// system the root file system is a read-only snapshot: there, the
/// package's own directories (`/.openshell/…`, `/sandbox`) are there, and
/// one that is missing could only be made in a new snapshot.
fn mount_point_error(directory: &str, error: &std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::ReadOnlyFilesystem {
        format!(
            "mount point {directory}: missing, and its file system is read-only (a transactional system?): \
             the package openshell-driver-systemd ships it; install the package again, in a new snapshot"
        )
    } else {
        format!("mount point {directory}: {error}")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::os::unix::fs::MetadataExt;

    use super::*;

    const ID: &str = "d6ac9d5f-60aa-4fcb-b050-9a0a4b134205";

    /// A mount point missing on a read-only root says what to do, and
    /// which one; another error names the mount point too.
    #[test]
    fn mount_point_error_names_the_read_only_root() {
        let readonly = std::io::Error::from(std::io::ErrorKind::ReadOnlyFilesystem);
        let message = mount_point_error("/sandbox", &readonly);
        assert!(
            message.starts_with("mount point /sandbox: missing"),
            "{message}"
        );
        assert!(message.contains("install the package again"), "{message}");
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let message = mount_point_error("/.openshell/channel", &denied);
        assert!(
            message.starts_with("mount point /.openshell/channel: "),
            "{message}"
        );
        assert!(!message.contains("read-only"), "{message}");
    }

    fn helper(state_dir: &Path) -> Helper {
        // Tests do not run as root: everything is the test's own.
        Helper {
            state_dir: state_dir.to_path_buf(),
            unit_dir: state_dir.join("units"),
            categories: false,
            owner_uid: nix::unistd::geteuid().as_raw(),
            owner_gid: nix::unistd::getegid().as_raw(),
            uid_range: UidRange {
                first: 70000,
                last: 70002,
            },
            userdb_dir: state_dir.join("userdb"),
        }
    }

    /// Makes a sandbox's directory with the test's own user as the
    /// sandbox's: a test cannot give files to another.
    fn claim(state_dir: &Path, sandbox_id: &str) {
        let dir = state_dir.join(sandbox_id);
        std::fs::create_dir(&dir).expect("sandbox directory");
        std::fs::write(dir.join("uid"), nix::unistd::geteuid().to_string()).expect("uid");
    }

    fn provision(token: &str) -> Provision {
        Provision {
            sandbox_id: ID.into(),
            name: "demo".into(),
            workspace: "default".into(),
            bootstrap: "{\"bootstrap\":true}".into(),
            server_certificate: "certificate".into(),
            server_key: "key".into(),
            runtime_descriptor: "{\"descriptor\":true}".into(),
            auth_bundle: "{\"auth\":true}".into(),
            sandbox_token: token.into(),
            gateway_ca: "ca".into(),
            environment: BTreeMap::from([("OPENSHELL_SANDBOX".to_string(), "demo".to_string())]),
        }
    }

    fn mode(path: &Path) -> u32 {
        std::fs::symlink_metadata(path).expect("stat").mode() & 0o7777
    }

    #[test]
    fn unit_states_combine_to_one() {
        for (supervisor, workload, expected) in [
            ("active", "active", UnitState::Active),
            ("inactive", "inactive", UnitState::Inactive),
            ("unknown", "inactive", UnitState::Inactive),
            ("failed", "inactive", UnitState::Failed),
            ("active", "failed", UnitState::Failed),
            ("active", "inactive", UnitState::Failed),
            ("active", "activating", UnitState::Transitioning),
            ("deactivating", "inactive", UnitState::Transitioning),
        ] {
            assert_eq!(
                unit_state(supervisor, workload),
                expected,
                "{supervisor}, {workload}"
            );
        }
    }

    #[test]
    fn launch_files_get_their_places_and_modes() {
        let state = tempfile::tempdir().expect("state directory");
        // The sandbox's directory is root's in production; here it is the
        // test's.
        claim(state.path(), ID);
        let helper = helper(state.path());
        helper
            .write_launch_files(&provision("token-one"))
            .expect("write launch files");
        let dir = state.path().join(ID);
        for (path, expected) in [
            ("channel", 0o755),
            ("channel/sandbox", 0o711),
            ("channel/sandbox/bootstrap.json", 0o600),
            ("channel/sandbox/server.key", 0o600),
            ("channel/control", 0o755),
            ("control", 0o1777),
            ("supervisor", 0o700),
            ("supervisor/auth.json", 0o600),
            ("etc", 0o700),
            ("etc/sandbox.jwt", 0o600),
            ("supervisor.env", 0o600),
            ("resolv.conf", 0o644),
            ("workspace", 0o700),
            ("userdb", 0o755),
            ("meta.json", 0o600),
        ] {
            assert_eq!(mode(&dir.join(path)), expected, "{path}");
        }
        assert_eq!(
            std::fs::read_to_string(dir.join("supervisor.env")).expect("environment"),
            "OPENSHELL_SANDBOX=\"demo\"\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("etc/sandbox.jwt")).expect("token"),
            "token-one"
        );
        let metadata = helper.metadata(ID).expect("metadata");
        assert_eq!(
            (metadata.name.as_str(), metadata.workspace.as_str()),
            ("demo", "default")
        );
    }

    #[test]
    fn a_second_launch_replaces_the_files_and_keeps_the_workspace() {
        let state = tempfile::tempdir().expect("state directory");
        claim(state.path(), ID);
        let helper = helper(state.path());
        helper
            .write_launch_files(&provision("token-one"))
            .expect("first launch");
        let dir = state.path().join(ID);
        std::fs::write(dir.join("workspace/kept.txt"), "kept").expect("workspace file");
        std::fs::write(dir.join("control/control.sock"), "stale").expect("stale socket");
        helper
            .write_launch_files(&provision("token-two"))
            .expect("second launch");
        assert_eq!(
            std::fs::read_to_string(dir.join("etc/sandbox.jwt")).expect("token"),
            "token-two"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("workspace/kept.txt")).expect("workspace file"),
            "kept"
        );
        assert!(!dir.join("control/control.sock").exists());
    }

    #[test]
    fn a_launch_without_a_token_keeps_the_one_before() {
        let state = tempfile::tempdir().expect("state directory");
        claim(state.path(), ID);
        let helper = helper(state.path());
        let error = helper
            .write_launch_files(&provision(""))
            .expect_err("no token to keep");
        assert!(error.contains("no gateway token"), "{error}");
        helper
            .write_launch_files(&provision("token-one"))
            .expect("first launch");
        helper
            .write_launch_files(&provision(""))
            .expect("second launch");
        assert_eq!(
            std::fs::read_to_string(state.path().join(ID).join("etc/sandbox.jwt")).expect("token"),
            "token-one"
        );
    }

    const OTHER_ID: &str = "0b0e0f52-6a57-4b3c-9d0e-6f1f1f7a2c11";

    #[test]
    fn a_sandbox_keeps_its_categories_and_shares_them_with_no_other() {
        let state = tempfile::tempdir().expect("state directory");
        let mut helper = helper(state.path());
        helper.categories = true;
        for id in [ID, OTHER_ID] {
            claim(state.path(), id);
        }
        let mut other = provision("token-two");
        other.sandbox_id = OTHER_ID.to_string();

        let first = helper
            .write_launch_files(&provision("token-one"))
            .expect("first launch")
            .expect("categories");
        let second = helper
            .write_launch_files(&other)
            .expect("the other sandbox")
            .expect("categories");
        let again = helper
            .write_launch_files(&provision(""))
            .expect("second launch")
            .expect("categories");

        assert_eq!(first, again);
        assert_ne!(first, second);
        assert!(first.0 < first.1 && first.1 < Categories::COUNT);
        assert_eq!(
            helper.metadata(ID).expect("metadata").categories,
            Some(first)
        );
        assert_eq!(first.level(), format!("s0:c{},c{}", first.0, first.1));
    }

    #[test]
    fn without_the_policy_module_a_sandbox_has_no_categories() {
        let state = tempfile::tempdir().expect("state directory");
        claim(state.path(), ID);
        let helper = helper(state.path());
        let categories = helper
            .write_launch_files(&provision("token-one"))
            .expect("launch");
        assert_eq!(categories, None);
        let metadata =
            std::fs::read_to_string(state.path().join(ID).join("meta.json")).expect("metadata");
        assert!(!metadata.contains("categories"), "{metadata}");
    }

    #[test]
    fn metadata_from_before_categories_is_still_read() {
        let metadata: Metadata =
            serde_json::from_str(r#"{"name":"one","workspace":"default"}"#).expect("metadata");
        assert_eq!(metadata.categories, None);
    }

    #[test]
    fn drop_ins_start_both_units_at_the_sandbox_level() {
        let state = tempfile::tempdir().expect("state directory");
        let helper = helper(state.path());
        let categories = Some(Categories(7, 512));

        assert!(
            helper
                .write_drop_ins(ID, 70001, categories)
                .expect("written")
        );
        assert!(
            !helper
                .write_drop_ins(ID, 70001, categories)
                .expect("unchanged")
        );
        for (kind, domain) in CONFINED_UNITS {
            let dir = helper
                .unit_dir
                .join(format!("openshell-sandbox-{kind}@{ID}.service.d"));
            assert_eq!(
                std::fs::read_to_string(dir.join("50-selinux.conf")).expect("drop-in"),
                format!("[Service]\nSELinuxContext=system_u:system_r:{domain}:s0:c7,c512\n")
            );
            let user = std::fs::read_to_string(dir.join("40-user.conf")).expect("drop-in");
            assert!(user.starts_with("[Service]\nUser=70001\nGroup="), "{user}");
        }
        assert!(helper.remove_drop_ins(ID));
        assert!(!helper.remove_drop_ins(ID));
    }

    #[test]
    fn without_the_policy_module_the_drop_ins_still_name_the_user() {
        let state = tempfile::tempdir().expect("state directory");
        let helper = helper(state.path());
        assert!(helper.write_drop_ins(ID, 70001, None).expect("written"));
        for (kind, _) in CONFINED_UNITS {
            let dir = helper
                .unit_dir
                .join(format!("openshell-sandbox-{kind}@{ID}.service.d"));
            assert!(dir.join("40-user.conf").exists());
            assert!(!dir.join("50-selinux.conf").exists());
        }
    }

    const THIRD_ID: &str = "7c1d2e3f-4a5b-4c6d-8e9f-0a1b2c3d4e5f";
    const FOURTH_ID: &str = "9a8b7c6d-5e4f-4a3b-9c2d-1e0f9a8b7c6d";

    #[test]
    fn every_sandbox_gets_a_user_of_its_own_and_keeps_it() {
        let state = tempfile::tempdir().expect("state directory");
        let helper = helper(state.path());
        let first = helper.reserve(ID).expect("first");
        let second = helper.reserve(OTHER_ID).expect("second");
        assert_eq!((first, second), (70000, 70001));
        assert_eq!(helper.reserve(ID).expect("again"), first);
        assert_eq!(helper.user_of(ID), Some(first));
        assert_eq!(mode(&state.path().join(ID).join("uid")), 0o600);

        // The range is used up with the third; a user comes back with
        // its sandbox's directory.
        assert_eq!(helper.reserve(THIRD_ID).expect("third"), 70002);
        let error = helper.reserve(FOURTH_ID).expect_err("none left");
        assert!(error.contains("no free user ID"), "{error}");
        assert!(!state.path().join(FOURTH_ID).exists());
        std::fs::remove_dir_all(state.path().join(OTHER_ID)).expect("delete");
        assert_eq!(helper.reserve(FOURTH_ID).expect("fourth"), second);
    }

    #[tokio::test]
    async fn a_reserved_user_alone_is_not_yet_a_sandbox() {
        let state = tempfile::tempdir().expect("state directory");
        let helper = helper(state.path());
        let reserved = helper
            .handle(Request::Reserve {
                sandbox_id: ID.to_string(),
            })
            .await;
        assert!(
            matches!(reserved, Response::Reserved { uid: 70000, .. }),
            "{reserved:?}"
        );
        assert_eq!(
            helper
                .handle(Request::Status {
                    sandbox_id: ID.to_string()
                })
                .await,
            Response::NotFound
        );
        assert_eq!(
            helper.handle(Request::List).await,
            Response::Listed {
                sandboxes: Vec::new()
            }
        );
    }

    #[test]
    fn launch_files_are_not_written_for_a_sandbox_without_a_user() {
        let state = tempfile::tempdir().expect("state directory");
        std::fs::create_dir(state.path().join(ID)).expect("sandbox directory");
        let error = helper(state.path())
            .write_launch_files(&provision("token"))
            .expect_err("refused");
        assert!(error.contains("no user was reserved"), "{error}");
        // Nor for one whose file does not hold a number.
        std::fs::write(state.path().join(ID).join("uid"), "root").expect("uid");
        assert_eq!(helper(state.path()).user_of(ID), None);
    }

    #[test]
    fn a_range_leaves_out_what_the_system_keeps() {
        assert_eq!(
            "70000-79999".parse::<UidRange>(),
            Ok(UidRange {
                first: 70000,
                last: 79999
            })
        );
        assert!("2000-2999".parse::<UidRange>().is_ok());
        for range in [
            "",
            "70000",
            "79999-70000",
            "0-10",
            "999-2000",
            "60000-70000",
            "61184-65519",
            "65534-65534",
            "70000-4294967295",
            "a-b",
        ] {
            assert!(range.parse::<UidRange>().is_err(), "{range:?}");
        }
    }

    #[test]
    fn a_user_has_its_name_by_name_and_by_number() {
        let state = tempfile::tempdir().expect("state directory");
        let dir = state.path();
        let (owner, group) = (
            nix::unistd::geteuid().as_raw(),
            nix::unistd::getegid().as_raw(),
        );
        write_userdb(dir, 70001, owner, group).expect("written");
        write_userdb(dir, 70001, owner, group).expect("written again");
        let user: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join("70001.user")).expect("by number"),
        )
        .expect("a record");
        assert_eq!(user["userName"], "osb-70001");
        assert_eq!(user["uid"], 70001);
        assert_eq!(user["homeDirectory"], "/sandbox");
        assert_eq!(
            std::fs::read_link(dir.join("70001.user")).expect("a link"),
            Path::new("osb-70001.user")
        );
        assert_eq!(mode(&dir.join("osb-70001.user")), 0o644);
        assert!(dir.join("osb-70001.group").exists());
        remove_userdb(dir, 70001);
        assert_eq!(std::fs::read_dir(dir).expect("directory").count(), 0);
    }

    #[test]
    fn an_environment_that_does_not_fit_a_line_writes_nothing() {
        let state = tempfile::tempdir().expect("state directory");
        claim(state.path(), ID);
        let helper = helper(state.path());
        let mut bad = provision("token");
        bad.environment.insert(
            "OPENSHELL_LOG_LEVEL".to_string(),
            "info\nINJECTED=1".to_string(),
        );
        let error = helper.write_launch_files(&bad).expect_err("refused");
        assert!(error.contains("OPENSHELL_LOG_LEVEL"), "{error}");
        assert!(!state.path().join(ID).join("supervisor.env").exists());
    }

    #[test]
    fn a_supervisor_is_given_only_openshells_own_variables() {
        let state = tempfile::tempdir().expect("state directory");
        claim(state.path(), ID);
        let helper = helper(state.path());
        for name in [
            "LD_PRELOAD",
            "PATH",
            "OPENSHELL_SYSTEMD_SUPERVISOR_LANDLOCK",
            "openshell_sandbox",
        ] {
            let mut bad = provision("token");
            bad.environment.insert(name.to_string(), "x".to_string());
            let error = helper.write_launch_files(&bad).expect_err("refused");
            assert!(error.contains(name), "{error}");
            assert!(!state.path().join(ID).join("supervisor.env").exists());
        }
    }

    #[test]
    fn categories_outlive_a_helper_that_finds_no_policy_module() {
        let state = tempfile::tempdir().expect("state directory");
        claim(state.path(), ID);
        let mut helper = helper(state.path());
        helper.categories = true;
        let first = helper
            .write_launch_files(&provision("token-one"))
            .expect("first launch");
        assert!(first.is_some());
        helper.categories = false;
        let second = helper
            .write_launch_files(&provision(""))
            .expect("second launch");
        assert_eq!(first, second);
    }

    #[test]
    fn a_launch_that_fails_leaves_the_metadata_of_the_one_before() {
        let state = tempfile::tempdir().expect("state directory");
        claim(state.path(), ID);
        let mut helper = helper(state.path());
        helper.categories = true;
        let first = helper
            .write_launch_files(&provision("token-one"))
            .expect("first launch");
        let mut bad = provision("");
        bad.environment
            .insert("OPENSHELL_LOG_LEVEL".to_string(), "a\nb".to_string());
        helper.write_launch_files(&bad).expect_err("refused");
        assert_eq!(helper.metadata(ID).expect("metadata").categories, first);
    }

    #[test]
    fn a_kept_token_is_not_read_through_a_link() {
        let state = tempfile::tempdir().expect("state directory");
        let secret = state.path().join("elsewhere");
        std::fs::write(&secret, "not-a-token").expect("another file");
        let link = state.path().join("link");
        std::os::unix::fs::symlink(&secret, &link).expect("link");
        assert!(read_plain_file(&link).is_err());
        assert_eq!(read_plain_file(&secret).expect("plain file"), "not-a-token");
    }

    #[tokio::test]
    async fn a_request_with_a_path_for_an_id_is_refused_before_anything_runs() {
        let state = tempfile::tempdir().expect("state directory");
        let helper = helper(state.path());
        for sandbox_id in [
            "../../etc",
            "",
            "sandbox",
            "D6AC9D5F-60AA-4FCB-B050-9A0A4B134205",
        ] {
            for request in [
                Request::Reserve {
                    sandbox_id: sandbox_id.to_string(),
                },
                Request::Start {
                    sandbox_id: sandbox_id.to_string(),
                },
                Request::Delete {
                    sandbox_id: sandbox_id.to_string(),
                },
                Request::Status {
                    sandbox_id: sandbox_id.to_string(),
                },
            ] {
                assert!(
                    matches!(helper.handle(request).await, Response::Error { .. }),
                    "{sandbox_id:?}"
                );
            }
        }
    }

    #[tokio::test]
    async fn an_unknown_sandbox_is_not_found() {
        let state = tempfile::tempdir().expect("state directory");
        let helper = helper(state.path());
        for request in [
            Request::Start {
                sandbox_id: ID.to_string(),
            },
            Request::Stop {
                sandbox_id: ID.to_string(),
            },
            Request::Delete {
                sandbox_id: ID.to_string(),
            },
            Request::Status {
                sandbox_id: ID.to_string(),
            },
        ] {
            assert_eq!(helper.handle(request).await, Response::NotFound);
        }
        assert_eq!(
            helper.handle(Request::List).await,
            Response::Listed {
                sandboxes: Vec::new()
            }
        );
    }
}
