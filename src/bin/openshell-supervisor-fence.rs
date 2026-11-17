// SPDX-FileCopyrightText: Copyright (c) 2026 Daniel Rahn
// SPDX-License-Identifier: Apache-2.0

//! Runs a sandbox's supervisor behind a Landlock fence.
//!
//! `openshell-supervisor` is the stock program of `OpenShell`. It runs on
//! the host's network as the workloads' user and holds one sandbox's
//! credentials; every other sandbox's supervisor is the same user. This
//! launcher enters a fence that names what one supervisor needs, and then
//! becomes the supervisor: it stays in reach of its own sandbox's files
//! and of no other's.
//!
//! The network is not part of the fence: what a sandbox may reach is
//! `OpenShell`'s policy, which the supervisor is there to hold.

use std::ffi::OsString;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use openshell_driver_systemd::fence::{Fence, Mode, Outcome};

#[derive(Parser)]
#[command(name = "openshell-supervisor-fence")]
#[command(version = openshell_driver_systemd::VERSION)]
struct Args {
    /// What to do on a kernel that cannot hold the supervisor to its
    /// files: `required` does not start it, `best-effort` goes on with
    /// what the kernel offers, `off` asks for no fence.
    ///
    /// An argument only, set in the unit: the supervisor's environment
    /// comes with a launch, and a launch must not decide this.
    #[arg(long, value_enum, default_value = "required")]
    landlock: Mode,

    /// The supervisor's runtime directory: its sockets and the proxy's
    /// TLS files.
    #[arg(long)]
    runtime_dir: PathBuf,

    /// The supervisor and its arguments.
    #[arg(last = true, required = true)]
    command: Vec<OsString>,
}

/// What one sandbox's supervisor needs. The paths under `/.openshell` and
/// in `/etc/openshell` are those its unit binds its own sandbox's files
/// to; no other sandbox's are among them.
fn fence(runtime_dir: &Path) -> Fence {
    let paths = |paths: &[&str]| paths.iter().map(PathBuf::from).collect::<Vec<_>>();
    let read = paths(&[
        "/etc",
        "/proc",
        "/sys",
        "/dev/urandom",
        "/.openshell/channel",
        // Where /etc/resolv.conf points on the hosts this is for.
        "/run/netconfig",
        "/run/systemd/resolve",
        // Where the host's CA certificates are on SUSE systems:
        // /etc/ssl/ca-bundle.pem and /etc/ssl/certs point here. The
        // supervisor verifies the hosts a sandbox reaches with them.
        "/var/lib/ca-certificates",
        // The names of the sandboxes' users.
        "/run/userdb",
    ]);
    let mut read_write = paths(&[
        "/.openshell/supervisor",
        "/.openshell/channel/control",
        "/tmp",
        "/var/tmp",
        "/dev/null",
    ]);
    read_write.push(runtime_dir.to_path_buf());
    Fence {
        read_execute: paths(&["/usr"]),
        read,
        read_write,
        no_tcp: false,
    }
}

fn main() -> ExitCode {
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .without_time()
        .init();
    match fence(&args.runtime_dir).enter_as(args.landlock) {
        Ok(Outcome::None) if args.landlock != Mode::Off => {
            tracing::warn!("The supervisor runs without a Landlock fence");
        }
        Ok(_) => {}
        Err(message) => {
            eprintln!("openshell-supervisor-fence: {message}");
            return ExitCode::FAILURE;
        }
    }
    // Only returns if the supervisor could not be started.
    let error = std::process::Command::new(&args.command[0])
        .args(&args.command[1..])
        .exec();
    eprintln!(
        "openshell-supervisor-fence: {}: {error}",
        args.command[0].to_string_lossy()
    );
    ExitCode::from(127)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fence_names_this_sandbox_and_no_state_directory() {
        let fence = fence(Path::new("/run/openshell-sandbox/one"));
        assert!(
            fence
                .read_write
                .contains(&PathBuf::from("/run/openshell-sandbox/one"))
        );
        // Not the directory that holds every sandbox's files, nor every
        // supervisor's runtime directory.
        let all = fence
            .read
            .iter()
            .chain(&fence.read_write)
            .chain(&fence.read_execute);
        for path in all {
            assert_ne!(path, Path::new("/var/lib/openshell/systemd"), "{path:?}");
            assert_ne!(path, Path::new("/var/lib/openshell"), "{path:?}");
            assert_ne!(path, Path::new("/var"), "{path:?}");
            assert_ne!(path, Path::new("/run/openshell-sandbox"), "{path:?}");
            assert_ne!(path, Path::new("/run"), "{path:?}");
            assert_ne!(path, Path::new("/"), "{path:?}");
        }
        assert!(!fence.no_tcp);
    }

    #[test]
    fn the_fence_lets_the_supervisor_read_the_host_s_ca_certificates() {
        let fence = fence(Path::new("/run/openshell-sandbox/one"));
        assert!(fence.read.contains(&PathBuf::from("/etc")));
        assert!(
            fence
                .read
                .contains(&PathBuf::from("/var/lib/ca-certificates"))
        );
    }

    #[test]
    fn the_command_follows_two_dashes() {
        let args = Args::try_parse_from([
            "openshell-supervisor-fence",
            "--runtime-dir",
            "/run/x",
            "--",
            "/usr/libexec/openshell/openshell-supervisor",
            "--workdir",
            "/sandbox",
        ])
        .expect("arguments");
        assert_eq!(args.command.len(), 3);
        assert_eq!(args.landlock, Mode::Required);
        assert!(
            Args::try_parse_from(["openshell-supervisor-fence", "--runtime-dir", "/run/x"])
                .is_err()
        );
    }
}
