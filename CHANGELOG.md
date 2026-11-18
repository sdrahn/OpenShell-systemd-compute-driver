# Changelog

All notable changes to the OpenShell systemd compute driver. Versions
follow [Semantic Versioning](https://semver.org/).

The changes since the last release are not in this file: each is a
file in `changes.d/` ([changes.d/README.md](changes.d/README.md)),
added by the pull request that makes the change. `tools/changelog show`
prints them as the next release's section; a release writes them here.
Released sections stay as they are.

## v0.1.3 — 2026-11-18

A fix release: creating a sandbox no longer fails now and then with
"ContainerStopped" while its units are being started. After the
update, restart the driver as the gateway's user (`systemctl --user
restart openshell-driver-systemd.service`); sandboxes that run are not
touched.

### Fixed

- Creating a sandbox no longer fails now and then with "sandbox
  entered error phase while provisioning: ContainerStopped". While the
  driver launches a sandbox, its units are reported as starting; before,
  the driver's periodic check could report them as stopped between
  provisioning and start, and the gateway failed the creation.

## v0.1.2 — 2026-11-17

A fix release: a sandbox's supervisor reads the host's CA certificates,
so a host whose certificate a private CA in the host's trust store
signed can be reached from a sandbox. With 0.1.1 the supervisor fell
back to the CA certificates built into OpenShell on SLES and openSUSE.
Restart the sandboxes that run to give their supervisors the change.

### Fixed

- A sandbox's supervisor reads the host's CA certificates. Its
  Landlock fence let it read `/etc`, but on SLES and openSUSE
  `/etc/ssl/ca-bundle.pem` is a link into `/var/lib/ca-certificates`,
  so it fell back to the CA certificates built into OpenShell, and a
  host with a private CA, added to the host's trust store, could not be
  reached through it ("unknown certificate authority").

## v0.1.1 — 2026-11-17

A fix release: a sandbox's own variables (`--env`, `--env-from`, and a
template's) reach its workload, which 0.1.0 never gave them. Sandboxes
made with 0.1.0 get them at their next start.

### Fixed

- A sandbox's own variables (`--env`, `--env-from`, and a template's)
  reach its workload and the commands run in it. Before, the workload
  had only a `PATH`: `openshell-sandbox` takes them from the launch's
  boundary configuration, where the driver did not put them.

## v0.1.0 — 2026-11-17

The first release: the driver as a project of its own, after OpenShell
decided to take no further compute drivers into its tree. It runs the
sandboxes of a stock OpenShell gateway as systemd units on the
gateway's host, on SLES 16 and openSUSE Leap 16, and is built and
tested against OpenShell 0.1.8.

To know before using it:

- The guide is in `docs/user-guide`, and installed by
  `openshell-driver-systemd-docs`. Its chapter 5 has the security model
  and the known limits, its chapter 10 what the functional tests check
  and what none does.
- Every sandbox runs as a host user of its own, from the user IDs
  70000 to 79999. A host with accounts in that range sets another
  (`OPENSHELL_SYSTEMD_UID_RANGE`) before the first sandbox.
- The code was written and reviewed with coding agents
  (`docs/security-review-0.1.md`); no person has reviewed it yet. It
  was tested on openSUSE Leap 16.0 with SELinux enforcing, not on
  SLES 16 itself and not on a host without the SELinux module.
- There was no release before this one: entries below that say what
  was there "before" speak of development builds. Sandboxes made by
  such a build are deleted before this version is installed.

### Added

- A guide for administrators and for those who work in a sandbox, in
  `docs/user-guide`: what the driver is and when to use it, installing
  and setting up, what a workload can and cannot do, network policy
  with a worked example, the security model, operation, troubleshooting
  with the error messages, a reference of every setting, the helper
  protocol, and which statements the tests hold. The README was the
  only documentation before and is now an overview.
- The package `openshell-driver-systemd-docs` installs the guide and
  the security review in `/usr/share/openshell-driver-systemd/docs`,
  with a collection file for mcp-docs, which then serves them to the
  agents on the host.
- `openshell-driver-systemd`, an external compute driver for the
  OpenShell gateway that runs each sandbox as systemd units on the
  gateway's host, with its privileged helper
  `openshell-systemd-helper` and the unit templates.
- An SELinux policy module with a domain for a sandbox's workload and
  one for its supervisor.
- A package for SLES 16 and openSUSE Leap 16 that also holds
  `openshell-supervisor` and `openshell-sandbox` of the OpenShell
  release the driver is built against.
- The package `openshell-driver-systemd-tests` with the functional
  tests, for openQA and other test systems, and an openQA test module
  (`test/openqa`). The tests write every check as a result of its own,
  as TAP and JUnit, and `--offline` leaves out the checks that need the
  Internet.
- Releases: a tag `vX.Y.Z` on a branch `release-X.Y` publishes the
  package's source files (the driver's source, OpenShell's source, the
  crates of both, the spec) with their checksums, for a build service
  to take.

### Changed

- The helper's state directory and the mount points of the sandbox units
  are created by a `tmpfiles.d` entry of the package, where the helper
  created them when it started.
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

### Fixed

- The package builds on the Open Build Service: the spec names
  `pkgconfig(systemd)`, which its macros require in the build root.
  The build stopped with "Failed build dependencies" before.
- With the SELinux module, a workload can ask how full a file system is
  (`df`), which the policy denied.
- With the SELinux module, a workload can run a script or a program it
  wrote in its `/tmp`, which the policy denied; in its workspace it
  already could.

### Removed

- `openshell-driver-systemd --bind-address`: the driver no longer
  listens on TCP, where it served anyone on the host. `--bind-socket`
  is required.
- `OPENSHELL_SYSTEMD_SUPERVISOR_LANDLOCK`: the supervisor's fence is
  set with `--landlock` in the unit only.

### Security

- `openshell-systemd-helper` puts a Landlock fence around itself before
  it serves a request: its state directory, the sandbox units' drop-ins
  and its socket, and no TCP. It binds root, with or without the SELinux
  module. On a kernel that cannot do this (before Linux 6.2) the helper
  does not start; `--landlock best-effort` or `off` changes that.
- A sandbox's supervisor runs behind a Landlock fence that names its own
  sandbox's files. Every supervisor is the same user, so file modes
  alone let one read another sandbox's credentials; where the SELinux
  module is not loaded, nothing else stood in the way. The unit starts
  the stock `openshell-supervisor` through a new program,
  `openshell-supervisor-fence`.
- The helper gives a supervisor only OpenShell's own environment
  variables (`OPENSHELL_…`, not `OPENSHELL_SYSTEMD_…`): a launch can no
  longer switch off the supervisor's Landlock fence or have a library
  loaded into it.
- The supervisor's unit holds its own sandbox's files and runtime
  directory and no other's, and the workload's unit nothing of the
  host's `/run` and shared memory of its own.
- With the SELinux module, a workload's domain cannot read its
  supervisor's credentials even where it could reach them.
- The helper sets owners and modes on the files it opened, not by their
  paths; keeps few of root's capabilities; runs its programs by their
  full paths; and does not start where SELinux is on but a label cannot
  be read.
- A sandbox keeps its SELinux categories when a launch fails or the
  helper starts without finding the policy module.
- An error of a start no longer carries the units' log lines.
- With the SELinux module, every sandbox has a pair of categories of its
  own. Its supervisor, its workload and its files carry them, so the
  policy refuses one sandbox what is another's, where only the
  namespaces kept them apart before. A sandbox that exists gets its
  pair at its next launch.
- With the SELinux module, `openshell-systemd-helper` runs in a domain of
  its own, `openshell_systemd_helper_t`, where it ran in the domain of
  any unconfined service: it reaches the sandboxes' files and the sandbox
  units, and no network.
- With the SELinux module, the unit that holds a sandbox's network
  namespace runs in a domain of its own, `openshell_sandbox_netns_t`,
  which may write network settings of that namespace and nothing else.
  Without the module, too, the unit keeps one capability of root's and
  the host is read-only for it.
- The driver's user unit gains no privileges and makes no namespaces
  (`NoNewPrivileges=`, `RestrictNamespaces=` and others).
- Every sandbox has a host user of its own. The helper gives it the
  first free user ID of a range (`--uid-range`,
  `OPENSHELL_SYSTEMD_UID_RANGE`, 70000 to 79999 unless set) and keeps it
  for the sandbox's life; its supervisor, its workload and its files
  are that user's. One sandbox's user is refused another's files and
  cannot signal another's processes by the kernel's ordinary checks,
  also on a host without the SELinux module and also signal 0. Before,
  every sandbox ran as `openshell-sandbox`, and only namespaces, SELinux
  categories and the supervisor's fence kept them apart.

