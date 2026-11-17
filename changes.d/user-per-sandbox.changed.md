- A sandbox's user is named `osb-<UID>` on the host and in the sandbox,
  through a record in `/run/userdb` that systemd's user database reads.
  Nothing is written to `/etc/passwd`. `id -un` in a sandbox printed
  `openshell-sandbox` before. The package's `tmpfiles.d` entry makes
  `/run/userdb` and, where it is missing, the empty directory
  `/etc/userdb`, over which a workload's unit mounts its own record.
- A workload's unit no longer holds `/var/log/journal`: journald keeps
  a journal file for each sandbox's user, which that user could read.
- The helper's `--workload-user` (`OPENSHELL_SYSTEMD_WORKLOAD_USER`) is
  gone. The user `openshell-sandbox` stays as the one the unit
  templates name; it owns nothing.
- A policy may ask for the user and group `sandbox` only; a number is
  refused, since the helper chooses it.
- Sandboxes made by a build before this change have files of
  `openshell-sandbox` and do not start: delete them.
