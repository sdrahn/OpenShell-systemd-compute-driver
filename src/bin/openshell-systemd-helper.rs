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
use openshell_driver_systemd::fence::{Fence, Outcome, Requirement};
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
];
const MOUNT_POINT_FILES: &[&str] = &[
    "/etc/openshell/auth/sandbox.jwt",
    "/etc/openshell/tls/client/ca.crt",
];

/// The workload's resolver is `openshell-sandbox`'s policy DNS.
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

    /// The user every supervisor and workload runs as. The unit templates
    /// name the same user.
    #[arg(
        long,
        env = "OPENSHELL_SYSTEMD_WORKLOAD_USER",
        default_value = "openshell-sandbox"
    )]
    workload_user: String,

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
    landlock: Landlock,

    /// Enter the fence, try what it must refuse and what it must allow,
    /// print each result and exit: 0 if all are as they must be.
    #[arg(long)]
    self_test: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum)]
enum Landlock {
    Required,
    BestEffort,
    Off,
}

/// Where a sandbox's units get their drop-ins.
const UNIT_DIR: &str = "/run/systemd/system";

/// What the helper needs once it serves, and what the programs it runs
/// need (`systemctl`, `restorecon`, `chcon`, `stat`): everything else of
/// the host is out of its reach, as root too.
fn fence(state_dir: &Path, socket: &Path) -> Fence {
    let paths = |paths: &[&str]| paths.iter().map(PathBuf::from).collect::<Vec<_>>();
    let mut read_write = vec![state_dir.to_path_buf(), PathBuf::from(UNIT_DIR)];
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
            "/dev/random",
            "/proc",
            "/sys",
            "/run",
            "/var/lib/selinux",
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
    workload_uid: u32,
    workload_gid: u32,
}

fn unit(kind: &str, sandbox_id: &str) -> String {
    format!("openshell-sandbox-{kind}@{sandbox_id}.service")
}

/// Creates a directory with exactly `mode`, owned by `uid:gid`.
fn create_directory(path: &Path, mode: u32, uid: u32, gid: u32) -> std::io::Result<()> {
    std::fs::DirBuilder::new().mode(mode).create(path)?;
    // The mode given at creation is subject to the umask.
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    std::os::unix::fs::chown(path, Some(uid), Some(gid))
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
    std::os::unix::fs::chown(path, Some(uid), Some(gid))
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
fn policy_module_in_use() -> bool {
    if !Path::new("/sys/fs/selinux/enforce").exists() {
        return false;
    }
    std::process::Command::new("stat")
        .args(["-c", "%C", "/usr/libexec/openshell/openshell-sandbox"])
        .output()
        .is_ok_and(|output| {
            String::from_utf8_lossy(&output.stdout).contains(":openshell_sandbox_exec_t:")
        })
}

/// Gives files a level, as `chcon -l` does.
fn set_level(level: &str, recursive: bool, paths: &[PathBuf]) -> std::result::Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut command = std::process::Command::new("chcon");
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
        let labelled = std::process::Command::new("stat")
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

    /// Writes the drop-ins that start a sandbox's units at its level.
    /// Returns whether systemd has to read its units again.
    fn write_drop_ins(
        &self,
        sandbox_id: &str,
        categories: Categories,
    ) -> std::result::Result<bool, String> {
        let mut changed = false;
        for (kind, domain) in CONFINED_UNITS {
            let dir = self.drop_in(kind, sandbox_id);
            let file = dir.join("50-selinux.conf");
            let content = format!(
                "[Service]\nSELinuxContext=system_u:system_r:{domain}:{}\n",
                categories.level()
            );
            if std::fs::read_to_string(&file).is_ok_and(|current| current == content) {
                continue;
            }
            std::fs::create_dir_all(&dir)
                .and_then(|()| std::fs::write(&file, content))
                .map_err(|error| format!("write {}: {error}", file.display()))?;
            changed = true;
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
        let environment = envfile::render(&provision.environment).map_err(|e| e.to_string())?;
        let categories = if self.categories {
            Some(self.categories_of(&provision.sandbox_id)?)
        } else {
            None
        };
        let metadata = serde_json::to_vec(&Metadata {
            name: provision.name.clone(),
            workspace: provision.workspace.clone(),
            categories,
        })
        .map_err(|error| error.to_string())?;
        let (uid, gid) = (self.workload_uid, self.workload_gid);
        let dir = self.sandbox_dir(&provision.sandbox_id);
        let sandbox_token = if provision.sandbox_token.is_empty() {
            std::fs::read_to_string(dir.join("etc/sandbox.jwt"))
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
            "meta.json",
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
        let workspace = dir.join("workspace");
        if !workspace.exists() {
            create_directory(&workspace, 0o700, uid, gid).map_err(|e| describe("workspace", e))?;
        }
        create_file(
            &dir.join("meta.json"),
            &metadata,
            0o600,
            self.owner_uid,
            self.owner_gid,
        )
        .map_err(|e| describe("meta.json", e))?;
        Ok(categories)
    }

    async fn systemctl(arguments: &[&str]) -> std::result::Result<String, String> {
        let output = tokio::process::Command::new("systemctl")
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
        match tokio::process::Command::new("systemctl")
            .args(["is-active", unit])
            .output()
            .await
        {
            Ok(output) => String::from_utf8_lossy(&output.stdout).trim().to_string(),
            Err(_) => "unknown".to_string(),
        }
    }

    /// What systemd says of a sandbox's units and their last log lines,
    /// for the error of a start that failed. A unit's log may hold what
    /// the supervisor or `openshell-sandbox` printed, never a credential:
    /// neither logs one.
    async fn diagnose(sandbox_id: &str) -> String {
        const LIMIT: usize = 6000;
        let units = [
            unit("netns", sandbox_id),
            unit("supervisor", sandbox_id),
            unit("workload", sandbox_id),
        ];
        let mut arguments = vec!["status", "--no-pager", "--full", "--lines=15"];
        arguments.extend(units.iter().map(String::as_str));
        // `systemctl status` exits non-zero for units that are not active.
        let status = match tokio::process::Command::new("systemctl")
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
            Request::Start { sandbox_id }
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
                uid: self.workload_uid,
                gid: self.workload_gid,
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
                // The drop-ins are gone after a reboot: every start writes
                // what is missing.
                if let Some(categories) = metadata.categories {
                    let reload = match self.write_drop_ins(&sandbox_id, categories) {
                        Ok(false) => Ok(String::new()),
                        Ok(true) => Self::systemctl(&["daemon-reload"]).await,
                        Err(message) => Err(message),
                    };
                    if let Err(message) = reload {
                        return Response::Error { message };
                    }
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
        let read = (&mut stream)
            .take(MAX_MESSAGE_BYTES)
            .read_to_end(&mut encoded)
            .await;
        let response = match read {
            Err(error) => Response::Error {
                message: format!("read the request: {error}"),
            },
            Ok(_) => match serde_json::from_slice::<Request>(&encoded) {
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
    let workload = user(&args.workload_user)?;
    if workload.uid.is_root() || workload.gid.as_raw() == 0 {
        return Err(miette!("the workload user must not be root"));
    }
    for directory in MOUNT_POINT_DIRECTORIES {
        std::fs::create_dir_all(directory).into_diagnostic()?;
    }
    for file in MOUNT_POINT_FILES {
        if !Path::new(file).exists() {
            create_file(Path::new(file), b"", 0o600, 0, 0).into_diagnostic()?;
        }
    }
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
    let categories = policy_module_in_use();
    if let Some(parent) = args.socket.parent() {
        std::fs::create_dir_all(parent).into_diagnostic()?;
    }
    // The fence, before there is a second thread: it holds the thread that
    // enters it and what that thread starts.
    let fenced = match args.landlock {
        Landlock::Off => Outcome::None,
        Landlock::Required => fence(&args.state_dir, &args.socket)
            .enter(Requirement::Required)
            .map_err(|message| miette!("{message}"))?,
        Landlock::BestEffort => fence(&args.state_dir, &args.socket)
            .enter(Requirement::BestEffort)
            .map_err(|message| miette!("{message}"))?,
    };
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
        workload_uid: workload.uid.as_raw(),
        workload_gid: workload.gid.as_raw(),
    };
    info!(
        socket = %args.socket.display(),
        allow_user = %args.allow_user,
        workload_user = %args.workload_user,
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::os::unix::fs::MetadataExt;

    use super::*;

    const ID: &str = "d6ac9d5f-60aa-4fcb-b050-9a0a4b134205";

    fn helper(state_dir: &Path) -> Helper {
        // Tests do not run as root: everything is the test's own.
        Helper {
            state_dir: state_dir.to_path_buf(),
            unit_dir: state_dir.join("units"),
            categories: false,
            owner_uid: nix::unistd::geteuid().as_raw(),
            owner_gid: nix::unistd::getegid().as_raw(),
            workload_uid: nix::unistd::geteuid().as_raw(),
            workload_gid: nix::unistd::getegid().as_raw(),
        }
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
        std::fs::create_dir(state.path().join(ID)).expect("sandbox directory");
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
        std::fs::create_dir(state.path().join(ID)).expect("sandbox directory");
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
        std::fs::create_dir(state.path().join(ID)).expect("sandbox directory");
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
            std::fs::create_dir(state.path().join(id)).expect("sandbox directory");
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
        std::fs::create_dir(state.path().join(ID)).expect("sandbox directory");
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
        let categories = Categories(7, 512);

        assert!(helper.write_drop_ins(ID, categories).expect("written"));
        assert!(!helper.write_drop_ins(ID, categories).expect("unchanged"));
        for (kind, domain) in CONFINED_UNITS {
            let file = helper.unit_dir.join(format!(
                "openshell-sandbox-{kind}@{ID}.service.d/50-selinux.conf"
            ));
            assert_eq!(
                std::fs::read_to_string(file).expect("drop-in"),
                format!("[Service]\nSELinuxContext=system_u:system_r:{domain}:s0:c7,c512\n")
            );
        }
        assert!(helper.remove_drop_ins(ID));
        assert!(!helper.remove_drop_ins(ID));
    }

    #[test]
    fn an_environment_that_does_not_fit_a_line_writes_nothing() {
        let state = tempfile::tempdir().expect("state directory");
        std::fs::create_dir(state.path().join(ID)).expect("sandbox directory");
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
