---
title: "10. What the tests hold"
description: Which statements of the OpenShell systemd compute driver guide are checked by the functional tests on an installed openSUSE Leap 16 system, section by section, which statements no test checks, and how to run the tests on a disposable machine.
tags: [tests, verification, limits]
---

# 10. What the tests hold

The functional tests create, use, stop, start and delete sandboxes
through a stock OpenShell gateway on an installed system, and try what
a workload, a supervisor and the helper must not reach. CI runs them on
every change, on the built packages, in an openSUSE Leap 16.0 VM with
SELinux enforcing. This chapter maps the guide's statements to them and
lists what no test shows.

## Statements a test checks {#held}

| Statement | Chapter | Test section |
|---|---|---|
| the packages install; the unit templates are valid; the programs carry their labels | 2 | the packages |
| the helper starts behind its fence and in its domain, with five capabilities; its self-test passes, and fails with the fence off | 2, 5 | the helper |
| the helper answers the gateway's user, not root, not another user; a path as sandbox ID is refused | 5, 9 | the helper |
| the mount points exist without the helper making them | 8 | the helper |
| the driver runs as the gateway user's service; its socket is 0600; the gateway names the driver | 2 | the driver and the gateway |
| a sandbox becomes ready; three units; namespaces (network, process IDs, IPC), domains, categories on processes and files | 1, 5 | a sandbox, the units |
| a sandbox has a user of its own from the range, not in `/etc/passwd`, known by name on the host and in the sandbox; both units and the sandbox's files are that user's | 1, 3, 5 | the units; what the workload can do; through the gateway |
| the supervisor's and the workload's units hold no other sandbox's files, no host `/run` | 5 | the units |
| a workload: no capabilities, no new privileges, seccomp on, loopback only; writes `/sandbox` and `/tmp`, not `/etc` or `/usr`; no `/etc/shadow`, no supervisor files or token; no mount, no namespace; no D-Bus, systemd or journal socket | 3, 5 | what the workload can do |
| a command runs through the gateway; an image is refused | 3 | through the gateway |
| git, C compiler, make, Python, loopback servers and low ports, Unix sockets, tar, scripts in `/tmp`, own processes, reading the host; no `systemctl`, no `journalctl`, no journal files | 3 | what an agent does |
| a restart of the driver leaves a sandbox's processes; after a restart of the gateway the sandbox is ready again | 3, 6 | restarts under a running sandbox |
| the named program reaches the allowed host; another host is refused; an unnamed copy is refused; named copies in `/sandbox` and `/tmp` reach it | 4 | a connection the policy allows |
| two sandboxes have different categories; outside the namespaces one's files are refused with the other's categories | 5 | two sandboxes |
| one sandbox's user, with no namespace, domain or fence in the way, cannot read another's workspace, token or runtime directory, and cannot signal its supervisor or workload, signal 0 included | 5 | two sandboxes |
| the supervisor's fence alone refuses root another sandbox's token, workspace and runtime directory, and lets it read the host's CA bundle | 4, 5 | two sandboxes |
| a sandbox reaches a host on the host's loopback whose certificate a private CA in the host's trust store signed | 4 | a host with a private CA |
| with another sandbox's categories a real signal to a supervisor is refused; a workload cannot signal or inspect supervisors or the other workload | 5 | two sandboxes |
| deleting a sandbox removes its files and units | 3, 6 | two sandboxes; stop, start, delete |
| a killed workload shows as `Error` | 3, 6 | a workload that dies |
| stop, start: the workspace is kept and written again, by the same user | 3 | stop, start, delete |
| a deleted sandbox's user has no name any more | 5, 6 | two sandboxes |
| nothing was denied by SELinux but what the tests provoke | 5 | SELinux denials |

The guide itself: `tools/check-docs` (in `make check`) holds its
structure and links, the functional tests that it is installed, and CI
that mcp-docs serves it from the installed package
(`test/docs/mcp-docs-smoke.sh`).

Unit tests (`make check`) hold, among others: the form of a sandbox ID,
that every sandbox gets a user of its own and keeps it, which ranges
are refused,
the environment's allowed names and quoting, that a sandbox keeps its
categories across launches, that the driver has no way in but its
socket, and what the supervisor's fence leaves out.

## Statements no test checks {#not-held}

Stated in this guide from the code, from systemd's or OpenShell's
documented behaviour, or as advice:

- **Other hosts**: SLES 16 itself; aarch64; a kernel without Landlock or
  with one before Linux 6.2 (`best-effort`); a host without the SELinux
  module or with SELinux off ([chapter 5](05-security.md#without-selinux)).
- **Life cycle**: a reboot between two starts of a sandbox; a restart
  of the helper under running sandboxes; that `/tmp` is empty after a
  stop; upgrading the packages; removing them; restoring a workspace
  from a backup; removing a forgotten sandbox's directory by hand.
- **Setup variants**: a gateway without TLS (no `OPENSHELL_SYSTEMD_TLS_CA`);
  `OPENSHELL_GRPC_ENDPOINT` and `OPENSHELL_GATEWAY_PORT` set to other
  values; a CA file read from the gateway user's home (the tests use a
  copy in `/etc`); OpenShell installed from a system's repositories
  rather than by its installer.
- **Limits and drop-ins**: `MemoryMax=`, `CPUQuota=`, a slice for all
  sandboxes, any change to the unit templates
  ([chapter 6](06-operation.md#limits)).
- **Users**: a range other than the default; a host whose
  `nsswitch.conf` has no `systemd` (users without names); a range that
  is used up or holds accounts of a directory service (a unit test
  holds the helper's choice among free IDs).
- **Refusals**: a policy that asks for a particular user or group (a
  unit test holds the driver's check; no sandbox is created with one); that
  a policy cannot widen the unit's read-only mounts; a workload trying
  `sudo`, a pseudo-terminal or `/dev/shm`.
- **OpenShell's features beyond the tests' scenarios**: providers and
  their credentials, port forwards, policies other than the example,
  what a sandbox without a policy may reach.
- **Troubleshooting**: most messages in
  [chapter 7](07-troubleshooting.md) are taken from the code and from
  systemd; few were provoked on a system.

## Running the tests {#run}

They are the package `openshell-driver-systemd-tests`. **They change
the system for good** (they install packages, create a user, set up a
gateway, restart services) and refuse to start without `--disposable`:

```shell
zypper -n in openshell-driver-systemd-tests openshell-driver-systemd-selinux openshell-driver-systemd-docs
/usr/libexec/openshell-driver-systemd/tests/openshell-setup.sh
/usr/libexec/openshell-driver-systemd/tests/run-tests.sh --disposable --results /var/tmp/results
```

| Option | Effect |
|---|---|
| `--disposable` | required: says the machine may be thrown away |
| `--results DIR` | the log, `results.tap` (TAP), `results.xml` (JUnit, one suite per section), the SELinux denials and the journals |
| `--offline` | leaves out the checks that reach `example.com`; they are then `SKIP` |
| `--selinux-discovery` | for work on the policy module: the domains are permissive and every denial is listed |

Each check prints `PASS`, `FAIL` or `SKIP`. The exit status is 1 if one
failed. From a checkout, `test/vm/run-vm.sh <rpm dir>` runs them in a
disposable Leap 16 VM, as CI does. `test/openqa/README.md` in the
source says how openQA runs them.
