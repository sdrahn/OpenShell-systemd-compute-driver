# The driver in openQA

The functional tests come as the package `openshell-driver-systemd-tests`.
They are meant for openQA and other test systems that test a product with
the packages of its repositories. This repository's own CI runs the same
installed script, so the two cannot drift apart.

## What the package holds

| Path | What |
|---|---|
| `/usr/libexec/openshell-driver-systemd/tests/run-tests.sh` | the tests (`test/vm/run-tests.sh` in the source) |
| `/usr/libexec/openshell-driver-systemd/tests/openshell-setup.sh` | sets up an OpenShell gateway as the tests expect it |
| `/usr/libexec/openshell-driver-systemd/tests/openshell-version` | the OpenShell release the tests are written for |

It requires `openshell-driver-systemd` and what the tests use: `audit`,
`curl`, `gcc`, `git-core`, `gzip`, `iproute2`, `make`, `policycoreutils`,
`procps`, `python3`, `tar` and `util-linux`. It recommends
`openshell-driver-systemd-selinux`; without the policy module the checks of
the SELinux domains fail. It also recommends
`openshell-driver-systemd-docs`; without it the checks of the guide are
`SKIP`.

## Running them

As root, on a system that is there for testing:

```bash
zypper -n in openshell-driver-systemd-tests openshell-driver-systemd-selinux
/usr/libexec/openshell-driver-systemd/tests/openshell-setup.sh --from-repositories
/usr/libexec/openshell-driver-systemd/tests/run-tests.sh --disposable --results /var/tmp/openshell-driver-systemd-tests
```

- **They change the system for good.** `openshell-setup.sh` installs Podman
  and OpenShell, creates the user `osh` with lingering and starts
  OpenShell's gateway as that user's service. `run-tests.sh` starts the
  helper and the driver, switches the gateway to the driver, and creates
  and deletes sandboxes. Without `--disposable` it refuses to start (exit
  status 2).
- **OpenShell.** `--from-repositories` installs the packages `openshell`
  and `openshell-gateway` from the system's repositories. Without it, the
  script fetches OpenShell's own installer and packages from the Internet,
  at the release in `openshell-version`; this repository's CI does that.
- **Results:** every check prints `PASS`, `FAIL` or `SKIP`, in sections
  (`== the helper`). With `--results DIR` the script writes `results.tap`
  (TAP) and `results.xml` (JUnit, one test suite per section) there, with
  the log, the SELinux denials and the units' journals. Exit status is 0
  when nothing failed, 1 otherwise.
- **The Internet.** The section "a connection the policy allows" reaches
  `example.com` through a sandbox's supervisor. `--offline` leaves those
  checks out; they are then `SKIP`. The tests also install a few tools
  with `zypper`, which the package already requires.
- **SELinux:** the tests expect it enforcing, as the products ship it, and
  the policy module loaded.
- **The kernel:** Landlock, as the driver needs it.
- **Time:** about ten minutes. A timeout of 45 minutes leaves room.

## What they cover

| Section | Checks |
|---|---|
| the packages | the packages and programs; the user the unit templates name; the unit templates; the policy module and the programs' labels; the guide and its collection file |
| the helper | it starts, behind its Landlock fence and in its SELinux domain; the fence alone, by the helper's own test; whom it answers and what it refuses |
| the driver and the gateway | the driver as the gateway user's service; its socket; the gateway with the driver |
| a sandbox, the units | a sandbox through the gateway; its three units, their domains, users, namespaces and categories; the supervisor behind its launcher |
| what the workload can do | no capabilities, no new privileges, loopback only; what it writes and cannot write; no D-Bus, no systemd, no journal |
| through the gateway | a command in the sandbox; an image is refused |
| what an agent does | git, a C compiler, make, Python, a server on loopback, archives, a script in `/tmp`, reading the host |
| restarts under a running sandbox | the driver and the gateway restart; the sandbox goes on or comes back |
| a connection the policy allows | the named program reaches the host and no other; an unnamed copy is refused; named copies in the workspace and `/tmp` reach it |
| two sandboxes | different users and categories; one sandbox's user against the other's files and processes; one sandbox's files against the other's categories; the supervisor's fence alone; signals and processes |
| a workload that dies; stop, start, delete | the gateway learns of it; the workspace is kept; nothing is left |
| SELinux denials | only those the tests provoke |

## An openQA test module

[openshell_driver_systemd.pm](openshell_driver_systemd.pm) is meant for
[os-autoinst-distri-opensuse](https://github.com/os-autoinst/os-autoinst-distri-opensuse)
as `tests/console/openshell_driver_systemd.pm`. It installs the tests'
package and the policy module, sets up OpenShell from the repositories,
runs the tests and hands the JUnit results to openQA (`parse_extra_log`),
so each check is a result of its own. On failure it uploads the logs, the
SELinux denials and the journals.

Schedule it after the system is installed and registered, with:

- the repositories that hold `openshell`, `openshell-gateway` and the
  driver's packages;
- KVM is not needed: the sandboxes are systemd units;
- `OPENSHELL_DRIVER_SYSTEMD_OFFLINE=1` where the worker has no way to the
  Internet;
- a timeout of 45 minutes for the module (`MAX_JOB_TIME` as the job needs).

In a YAML schedule:

```yaml
schedule:
  - installation/bootloader_start
  - boot/boot_to_desktop
  - console/openshell_driver_systemd
```

## What has and has not been run

This repository's CI runs `run-tests.sh` from the installed package on
every pull request, in a Leap 16 VM, with OpenShell from its own installer.
Not run there: the test module itself (there is no openQA instance in CI),
`openshell-setup.sh --from-repositories` (CI has no repository with
OpenShell's packages), and `--offline`.

## Adding tests

A feature that shows on an installed system gets checks in
`test/vm/run-tests.sh`, in the pull request that adds it (AGENTS.md):

```bash
section "my feature"
check "what must hold, in words" some_command args    # PASS or FAIL
refuse "what must not be possible" some_command args  # PASS if it fails
skip "what this run leaves out" "the reason"
```
