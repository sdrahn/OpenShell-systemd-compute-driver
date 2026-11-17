// SPDX-FileCopyrightText: Copyright (c) 2026 Daniel Rahn
// SPDX-License-Identifier: Apache-2.0

//! A Landlock fence a program puts around itself.
//!
//! Landlock binds every user, root too, and what it takes cannot be had
//! back. It holds the thread that asks for it and everything that thread
//! starts afterwards, so a program asks before it has a second thread.
//!
//! What it does not cover: it grants a directory, not names in it; it does
//! not mediate connecting to a Unix socket by its path, nor changes of a
//! file's owner, mode or label.

use std::path::{Path, PathBuf};

use landlock::{
    ABI, Access, AccessFs, AccessNet, BitFlags, CompatLevel, Compatible, PathBeneath, PathFd,
    Ruleset, RulesetAttr, RulesetCreatedAttr, RulesetStatus,
};

/// The file access rights asked for are those of this version of Landlock
/// (Linux 6.2): with it a file cannot be truncated or moved out of reach
/// of the rules either.
const FILES: ABI = ABI::V3;
/// TCP is covered from this version on (Linux 6.7).
const NETWORK: ABI = ABI::V4;

/// What stays in reach. A path that does not exist is left out.
#[derive(Clone, Debug, Default)]
pub struct Fence {
    /// Read, and run programs from.
    pub read_execute: Vec<PathBuf>,
    /// Read only.
    pub read: Vec<PathBuf>,
    /// Everything: read, write, create, remove.
    pub read_write: Vec<PathBuf>,
    /// No TCP connection out and no TCP port taken, where the kernel can
    /// hold a program to that.
    pub no_tcp: bool,
}

/// What to do on a kernel that cannot hold the program to its files.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Requirement {
    /// Fail.
    Required,
    /// Go on with what the kernel offers, down to nothing.
    BestEffort,
}

/// What the kernel took on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    /// Every right asked for is held to the fence.
    Enforced,
    /// Some are: an older Landlock.
    Partial,
    /// None is: no Landlock in this kernel.
    None,
}

fn describe(what: &str, error: impl std::fmt::Display) -> String {
    format!("Landlock: {what}: {error}")
}

/// The rights of `access` that a path can carry: a file takes only those
/// of a file.
fn fitting(path: &Path, access: BitFlags<AccessFs>) -> BitFlags<AccessFs> {
    if path.is_dir() {
        access
    } else {
        access & AccessFs::from_file(FILES)
    }
}

impl Fence {
    /// Puts the calling thread, and what it starts from now on, behind the
    /// fence. Also sets "no new privileges" for it, as Landlock asks of a
    /// program without `CAP_SYS_ADMIN`.
    pub fn enter(&self, requirement: Requirement) -> Result<Outcome, String> {
        let level = match requirement {
            Requirement::Required => CompatLevel::HardRequirement,
            Requirement::BestEffort => CompatLevel::BestEffort,
        };
        let mut ruleset = Ruleset::default()
            .set_compatibility(level)
            .handle_access(AccessFs::from_all(FILES))
            .map_err(|error| describe("file access", error))?;
        if self.no_tcp {
            // Never a reason to fail: the files are the fence, this is more.
            ruleset = ruleset
                .set_compatibility(CompatLevel::BestEffort)
                .handle_access(AccessNet::from_all(NETWORK))
                .map_err(|error| describe("network access", error))?;
        }
        let mut ruleset = ruleset
            .create()
            .map_err(|error| describe("create the rules", error))?;
        let read = AccessFs::ReadFile | AccessFs::ReadDir;
        let grants = [
            (&self.read_execute, AccessFs::from_read(FILES)),
            (&self.read, read),
            (&self.read_write, AccessFs::from_all(FILES)),
        ];
        for (paths, access) in grants {
            for path in paths {
                let Ok(fd) = PathFd::new(path) else {
                    continue;
                };
                ruleset = ruleset
                    .add_rule(PathBeneath::new(fd, fitting(path, access)))
                    .map_err(|error| describe(&path.display().to_string(), error))?;
            }
        }
        let status = ruleset
            .restrict_self()
            .map_err(|error| describe("enter", error))?;
        Ok(match status.ruleset {
            RulesetStatus::FullyEnforced => Outcome::Enforced,
            RulesetStatus::PartiallyEnforced => Outcome::Partial,
            RulesetStatus::NotEnforced => Outcome::None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `inside` on a thread of its own: a fence stays for good.
    fn fenced<T: Send + 'static>(
        fence: Fence,
        inside: impl FnOnce(Outcome) -> T + Send + 'static,
    ) -> T {
        std::thread::spawn(move || {
            let outcome = fence.enter(Requirement::BestEffort).expect("fence");
            inside(outcome)
        })
        .join()
        .expect("fenced thread")
    }

    #[test]
    fn only_what_the_fence_names_is_written_and_read() {
        let allowed = tempfile::tempdir().expect("allowed directory");
        let readable = tempfile::tempdir().expect("readable directory");
        let outside = tempfile::tempdir().expect("outside directory");
        std::fs::write(readable.path().join("file"), "read me").expect("readable file");
        std::fs::write(outside.path().join("file"), "not for you").expect("outside file");
        let fence = Fence {
            read_write: vec![allowed.path().to_path_buf()],
            read: vec![readable.path().to_path_buf()],
            ..Fence::default()
        };
        let (allowed, readable, outside) = (
            allowed.path().to_path_buf(),
            readable.path().to_path_buf(),
            outside.path().to_path_buf(),
        );

        let checked = fenced(fence, move |outcome| {
            if outcome == Outcome::None {
                // A kernel without Landlock: nothing to hold to.
                return false;
            }
            std::fs::write(allowed.join("new"), "written").expect("write inside");
            assert_eq!(
                std::fs::read_to_string(readable.join("file")).expect("read"),
                "read me"
            );
            let denied =
                |error: std::io::Error| error.kind() == std::io::ErrorKind::PermissionDenied;
            assert!(std::fs::write(readable.join("new"), "no").is_err_and(denied));
            assert!(std::fs::read_to_string(outside.join("file")).is_err_and(denied));
            assert!(std::fs::write(outside.join("new"), "no").is_err_and(denied));
            assert!(std::fs::remove_file(outside.join("file")).is_err_and(denied));
            true
        });
        // The thread of the test itself is not fenced.
        assert_eq!(
            std::fs::read_to_string(outside.join("file")).expect("outside the fence"),
            "not for you"
        );
        if !checked {
            eprintln!("this kernel has no Landlock: nothing was checked");
        }
    }

    #[test]
    fn a_path_that_does_not_exist_is_left_out() {
        let fence = Fence {
            read: vec![PathBuf::from("/nonexistent/for/this/test")],
            ..Fence::default()
        };
        fenced(fence, |_| ());
    }
}
