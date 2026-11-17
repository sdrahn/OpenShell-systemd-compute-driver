# Security review before 0.1.0

A review of everything in this repository before its first release: the
helper that runs as root, the driver, the Landlock fences, the unit
templates, the SELinux module, the packaging and the crates of the build.
It covers `main` at `de333ff` and the changes made with this review.

**Who reviewed.** The code was written with a coding agent, and so was
this review. To have a reader who had not written it, the repository was
given to a second agent with the threat model below and no conclusions,
with the instruction to read the code as it is and to say for each
finding whether it had followed the code path or only suspected it. Its
eleven findings are S1 to S11 here; each was checked against the code
before anything was changed, and the fixes were then run in CI. A person
has not reviewed the code. For the helper in particular that is still
worth doing.

## The threat model

1. A workload that is hostile or was taken over (an agent that was
   prompt-injected, a tool's malicious output), inside its sandbox.
2. One sandbox against another, and against the supervisors. Every
   supervisor and every workload runs as one user, `openshell-sandbox`.
3. A local user without privileges, against the helper and the driver.
4. The user the helper serves (the OpenShell gateway's and the driver's):
   what can that user make root do beyond what the helper is for. Every
   field of every request is taken as the attacker's.
5. A supervisor that was taken over.

## Where it stands

No finding gives any of the five a way to run code as root or to have
root write to a path of their choosing. The weaknesses were in what keeps
one sandbox from another, most of all on a host without the SELinux
module, and in two defaults.

| Finding | Severity | Status |
|---|---|---|
| S1 | medium | fixed: the supervisor's environment is held to OpenShell's own variables |
| S2 | medium | partly fixed: a supervisor's unit holds no other's files or sockets; signals between supervisors stay (#21) |
| S3 | medium | fixed: the driver has no TCP listener |
| S4 | medium | partly fixed: the workload has its own `/run` and shared memory; no resource limits |
| S5 | medium | fixed: the supervisor's credentials are security files |
| S6 | low | fixed: the categories no longer fail open |
| S7 | low | fixed: owners and modes on the opened file |
| S8 | low | partly fixed: capabilities, full paths; a drop-in for any unit stays |
| S9 | low | partly fixed: a time limit for a request; requests stay one at a time |
| S10 | low | fixed: no log lines in an error |
| S11 | low | partly fixed: OpenShell's source by its commit; no checksum |
| D1 | info | checked now, in CI; three advisories accepted with reasons |

## S1 — medium — a launch could switch off the supervisor's fence

*Where:* `src/envfile.rs`; `systemd/openshell-sandbox-supervisor@.service`;
`src/bin/openshell-supervisor-fence.rs`. *Who:* 4.

The helper wrote any syntactically valid variable name of a launch into
the supervisor's `EnvironmentFile=`. The unit's `ExecStart=` is the
launcher of the supervisor's Landlock fence, and the launcher read its
setting from `OPENSHELL_SYSTEMD_SUPERVISOR_LANDLOCK`: a launch with that
variable set to `off` ran the supervisor without its fence. `LD_PRELOAD`
would have run code in the launcher before the fence. Both contradict
this repository's own rule that no request decides a security property.

An API client could not reach this: the driver puts a sandbox's own
environment into one JSON value, never into names. The gateway's user
could.

*Fix:* the helper accepts only names that begin with `OPENSHELL_` and not
with `OPENSHELL_SYSTEMD_`, and the launcher's setting is an argument of
the unit only. A unit test holds `LD_PRELOAD`, `PATH` and the driver's
own prefix refused.

## S2 — medium — supervisors were kept apart by Landlock alone

*Where:* `systemd/openshell-sandbox-supervisor@.service`. *Who:* 5
against 2.

Every supervisor is the same user and saw every sandbox's directory and
every supervisor's runtime directory; file modes allow all of it (the
functional tests show one supervisor's user reading another's token with
the fence off). The fence refused the files. It does not cover what
Landlock does not mediate: connecting to another supervisor's sockets in
its runtime directory, and signals.

*Fixed:* the supervisor's unit has a file system of its own over
`/var/lib/openshell` and `/run/openshell-sandbox` and holds only its own
sandbox's mounts and runtime directory. The other supervisors' sockets
are not there to connect to. The tests list both from inside a running
supervisor's mount namespace.

*Stays:* supervisors share the host's process IDs and a user, so one can
signal another, and a supervisor that is killed takes its workload with
it (`BindsTo=`).

- With the SELinux module, a process in the supervisor's domain with
  another sandbox's categories is refused a real signal (tested with
  `SIGCONT`). Signal 0, which only asks whether a process is there, is
  delivered: the distribution's policy does not hold it to categories,
  and a module cannot add such a constraint.
- Without the module nothing stands in the way.

The fix is a user for every sandbox (#21). A process ID namespace for
the supervisor is not one: it has to see its workload's processes.

## S3 — medium — the driver listened on TCP unless told otherwise

*Where:* `src/main.rs`. *Who:* 3 and 5.

Without `--bind-socket` the driver served the gateway's interface on
`127.0.0.1:50062` to whoever connected: listing, stopping and deleting
sandboxes (a workspace is removed as root) and creating one. The
packaged unit always passed a socket, so this was the program's default,
not the package's.

*Fix:* `--bind-socket` is required and the TCP mode is removed. Only the
user the driver runs as is served on the socket.

## S4 — medium — the workload's unit shared more of the host than it needed

*Where:* `systemd/openshell-sandbox-workload@.service`. *Who:* 1, and 2
between workloads.

- The host's `/run` was in the workload's mount namespace, with the
  sockets of D-Bus and the journal. A network namespace does not keep a
  Unix socket away; only the SELinux module refused them.
- Shared memory and message queues were the host's, for every workload
  under one user: a way from one sandbox to another.

*Fixed:* a file system of the unit's own over `/run`, and an IPC
namespace of the sandbox's own (the namespace unit's, which the workload
joins). The tests look into a running workload's namespaces for both.

*Stays:*

- No limit on memory, CPU or disk: a workload can fill its workspace's
  file system or exhaust memory. `TasksMax=` is the only limit. Listed
  as not supported in the README.
- The workload's unit binds the channel directory read-write. It has to:
  `openshell-sandbox` removes its bootstrap file and key after reading
  them.

## S5 — medium — the workload's domain could read its supervisor's credentials

*Where:* `selinux/openshell_systemd.te`. *Who:* 1.

The workload's domain reads "all files but the security files"
(`files_read_non_security_files`), and the type of a supervisor's launch
files was an ordinary file type. A workload and its supervisor have the
same categories, so those did not help: only the workload's mount
namespace hid the token and the authentication bundle.

*Fix:* `openshell_supervisor_conf_t` is a security file
(`files_security_file`). The supervisor's and the helper's rules name
the type and are unchanged.

*Not confirmed on a system:* that the macro on Leap 16's policy has
exactly this extent was read from knowledge of the reference policy, not
from the policy's sources; the functional tests pass with the change,
but none tries this read, since the mount namespace refuses it first.

*Left as it is:* the workload's domain may connect to and take any TCP
port. Its network namespace has only loopback; the rule is no second
fence if that namespace were ever missing.

## S6 — low — the categories failed open

*Where:* the helper: `policy_module_in_use`, `write_launch_files`.

Whether sandboxes get categories was decided once, when the helper
started, by reading a program's label. If that failed for any reason the
helper went on without categories, with one log field to show it. A
later launch then wrote "no categories" over a sandbox's pair, and a
launch that failed midway forgot the pair, since the metadata was
removed first and written last.

*Fix:* where SELinux is on and the label cannot be read, the helper does
not start. A sandbox that has a pair keeps it whatever the helper found
at its start. The metadata is written beside the old and renamed over it
as the last step. Unit tests hold the last two.

## S7 — low — root set owners by path in directories of the workloads' user

*Where:* the helper: `create_file`, `create_directory`, the kept token.

Three directories of a launch belong to the workloads' user before the
helper creates their files, and the helper then changed a file's owner by
its path. A process of that user with a writable view of the directory
could have put a symbolic link there between the two steps and had root
give it any file.

As shipped nothing had such a view: the sandbox's units are stopped
first, the directories are new, and no other unit sees them writable. On
a host without SELinux that rested on one setting of one unit.

*Fix:* owner and mode are set on the file or directory that was opened
(`fchown`, with `O_NOFOLLOW` for directories), and the token of an
earlier launch is read only from a regular file.

## S8 — low — a helper that was taken over is not contained

*Where:* the helper's fence; `selinux/openshell_systemd.te`.

The helper writes drop-ins among systemd's runtime units, for a
sandbox's categories. Its fence grants that directory, and the policy
lets it create a directory of any name there: a drop-in for any unit,
which takes effect when someone restarts that unit. Neither Landlock nor
a policy module can hold it to names. No field of a request reaches a
drop-in's path or content.

*Fixed in passing:* the helper keeps five of root's capabilities (those
for files of another user), has no home directories and no other kind of
socket than a local one, and runs `systemctl`, `chcon` and `stat` by
their full paths.

*Stays:* the drop-ins. There is no way in systemd to give a unit its
level without one. The domain and the fence bound a mistake in the
helper; they do not make a helper that runs as root harmless.

## S9 — low — one client could hold up the helper

*Where:* the helper: `serve`.

Requests are served one at a time, and reading a request had no time
limit: a connection of the allowed user that sent nothing blocked every
stop and delete.

*Fixed:* a request has 30 seconds to arrive.

*Stays:* requests are still served one at a time, and a stop waits for
systemd, which waits for a workload that ignores the signal to stop. The
only client is the driver, as the gateway's user.

## S10 — low — an error carried the units' log lines

*Where:* the helper: `diagnose`.

The error of a start that failed carried the last lines of the three
units' journals back to the gateway and its caller: what a workload
printed, and whatever the supervisor logged.

*Fix:* the error carries the units' states and results. The lines are in
the journal, for the administrator.

## S11 — low — what the package builds is not pinned by the lock file alone

*Where:* `packaging/suse/make-sources`; the spec.

OpenShell's source was fetched by its tag, which can be moved, and the
package builds the driver against that source instead of the copies the
lock file pins, without `--locked`.

*Fixed:* the source is fetched by the commit that `Cargo.lock` names for
the release, and the script fails if the lock file is at another
release.

*Stays:* there is no checksum of the archive in the repository, so its
content rests on the host it comes from; and the build is not `--locked`
for OpenShell's own crates, for the reason `packaging/suse/README.md`
gives. Every other crate stays as locked.

## D1 — info — the build's crates were not checked against an advisory database

*Fix:* the workflow `Vulnerabilities` runs `cargo audit` on every change
and every week, and fails on a finding that `.github/audit-known` does
not list with its reason.

*Its first run (2026-11-17), 443 crates:*

| Advisory | Crate | What was done |
|---|---|---|
| RUSTSEC-2026-0486 | `rustix` 1.1.4, unsound | updated to 1.1.6 |
| RUSTSEC-2026-0221 | `event-listener` 5.4.1, unsound | updated to 5.4.2 |
| RUSTSEC-2026-0459 | `libsqlite3-sys` 0.30.1, a vulnerability of SQLite | accepted: in the lock file for a feature the driver does not build (`cargo tree -i` shows no user) |
| RUSTSEC-2024-0449 | `unsafe-libyaml`, unmaintained | accepted: in the lock file only, not built |
| RUSTSEC-2026-0543 | `chrono`, unmaintained | accepted: comes with OpenShell's `openshell-ocsf`; no fixed version |

*Not covered:* the crates of `openshell-supervisor` and
`openshell-sandbox`, which the package builds from OpenShell's source
and OpenShell's lock file. Those are OpenShell's to check; the driver's
own lock file now differs from it in the two updated crates.

## Checked and sound

- **The protocol** (`src/protocol.rs`): a request names no path, no unit
  and no command. A sandbox's ID is checked to be a canonical lower-case
  UUID before any use of the file system or of `systemctl`, for every
  kind of request, and again when sandboxes are listed.
- **The environment file:** values are quoted as systemd reads them, and
  one with a control character is refused, so no line and no setting of
  a unit can be added. A sandbox's name and workspace reach only JSON.
- **Removing a sandbox:** the removal does not follow symbolic links, nor
  do `chcon -R` and `restorecon -R`. A workspace with content is gone
  through only when its directory lacks the sandbox's level.
- **A second launch:** everything but the workspace is removed and made
  anew, each file created and never opened if it exists.
- **The helper's socket:** mode 0600, the allowed user's, in a directory
  of root's; the peer's user is checked after accepting. The user to
  serve has no default. A workload user that is root is refused.
- **Credentials:** files of mode 0600 in directories of 0700 or 0711;
  the request's printed form leaves them out, and no request is logged.
- **Sizes:** a message is at most 4 MiB, on both sides.
- **The fences:** a path that is left out only makes a fence tighter.
  Behind a fence, Landlock also refuses one supervisor another's
  `/proc/<pid>/root` and environment.
- **The namespace's unit:** nothing of a request reaches it; one
  capability.
- **The package's scriptlets:** the distribution's macros only. In `/etc`
  the package's `tmpfiles.d` entry makes two directories and two empty
  files to mount on, where they are missing; nothing there is changed.

## What the tests do not show

- A kernel without Landlock, or with one before Linux 6.2.
- A host without the SELinux module. Several findings above are worse
  there, and no run shows the units on such a host.
- A reboot between two starts of a sandbox, when the drop-ins are gone
  and written again.
- SLES 16 itself: the tests run on Leap 16.0.
- What a supervisor needs beyond the tests' scenarios (a provider's
  files, a port forward): the kernel on Leap 16 logs no Landlock denial,
  so a need the tests do not exercise fails without a trace.
- The openQA test module, and OpenShell from a system's repositories
  (`test/openqa/README.md`).

## Since the review

- **A user for every sandbox (#21).** What S2 left, signals between
  supervisors, is closed: each sandbox's supervisor and workload run as
  a host user of that sandbox alone, so the kernel refuses one sandbox's
  user another's processes, signal 0 included, and another's files,
  also on a host without the SELinux module. The functional tests try
  both from outside every namespace, domain and fence. A supervisor and
  its own workload are still one user.
