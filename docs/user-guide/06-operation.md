---
title: "6. Operating the driver"
description: Day-to-day administration of the OpenShell systemd compute driver - finding a sandbox's units and files, reading the logs, limiting memory and CPU with drop-ins, changing the unit templates, what restarts and reboots do, disk space, backups and the audit log.
tags: [administrator, operation, systemd, logs, limits]
---

# 6. Operating the driver

Sandboxes are created and removed through the gateway only. What the
administrator of the host does is look at them with systemd's own
tools, set limits, and keep the two services running.

## The services {#services}

| Service | Manager | Start |
|---|---|---|
| `openshell-systemd-helper.service` | the system's | `systemctl enable --now …` |
| `openshell-driver-systemd.service` | the gateway user's (`systemctl --user`) | enabled for that user; ordered before `openshell-gateway.service` |

Both restart themselves after a failure, five seconds later.

## Finding a sandbox's units {#units}

A sandbox's ID is in `openshell sandbox get NAME` (`Id:`). Its units:

```shell
systemctl list-units 'openshell-sandbox-*'
systemctl status 'openshell-sandbox-*@<ID>.service'
systemd-cgls -u openshell-sandbox-workload@<ID>.service     # the workload's processes
```

In the other direction, a sandbox's name and OpenShell workspace are in
`/var/lib/openshell/systemd/<ID>/meta.json`.

Do not start, stop or restart a sandbox's units by hand. The gateway
does not learn why, and a start by hand uses launch files whose
credentials may have run out. Use `openshell sandbox stop` and `start`.
Stopping the supervisor's unit stops the workload's with it
(`BindsTo=`); the namespace's unit stops when nothing needs it.

## Reading the logs {#logs}

| What | Command |
|---|---|
| the helper | `journalctl -u openshell-systemd-helper.service` |
| the driver | `journalctl --user -u openshell-driver-systemd.service`, as the gateway's user |
| a sandbox's supervisor | `journalctl -u openshell-sandbox-supervisor@<ID>.service` |
| a sandbox's workload | `journalctl -u openshell-sandbox-workload@<ID>.service` |
| SELinux denials | `ausearch -m avc -ts recent`, or `grep 'avc:  denied' /var/log/audit/audit.log` |

What a workload prints is in its unit's journal: readable by the
host's administrators, and not sent back to the gateway with an error.
The level of the helper's and the driver's own lines is
`OPENSHELL_LOG_LEVEL` (`info` by default).

Landlock denials are not logged by the kernel of Leap 16.0. A program
behind a fence that is refused a file sees "Permission denied" and
nothing appears in a log.

## The files of a sandbox {#files}

`/var/lib/openshell/systemd/<ID>/`, written by the helper:

| Path | Owner, mode | Seen by |
|---|---|---|
| `meta.json` (name, workspace, categories) | root | the helper |
| `supervisor.env` (the supervisor's environment) | root, 0600 | systemd |
| `resolv.conf` | root | the workload, as `/etc/resolv.conf` |
| `channel/sandbox/` (bootstrap file, TLS identity) | `openshell-sandbox`, 0711 | both; read-only for the supervisor |
| `control/` (the control socket) | root, 1777 | both |
| `supervisor/` (runtime descriptor, authentication bundle) | `openshell-sandbox`, 0700 | the supervisor |
| `etc/` (gateway token `sandbox.jwt`, gateway CA `ca.crt`) | `openshell-sandbox`, 0700 | the supervisor |
| `workspace/` | `openshell-sandbox`, 0700 | the workload, as `/sandbox` |

`/run/openshell-sandbox/<ID>/` is the supervisor's runtime directory
(its sockets, its proxy's keys), gone when it stops.

Do not edit these files. Each launch writes all but the workspace anew.

## Limiting memory, CPU and processes {#limits}

The driver takes no resource requests from a sandbox. The only limit in
the templates is `TasksMax=2048` for a workload. Set others as with any
unit, in a drop-in for the template, which then holds for every
sandbox:

```shell
systemctl edit openshell-sandbox-workload@.service
```

```ini
[Service]
MemoryMax=4G
MemorySwapMax=0
CPUQuota=200%
TasksMax=1024
```

It applies to a sandbox at its next start. For one sandbox, edit
`openshell-sandbox-workload@<ID>.service` instead; that drop-in stays
after the sandbox is deleted, so remove it then.

A limit for all sandboxes together: put the units into a slice of their
own (`Slice=openshell-sandboxes.slice` in the drop-in) and set the
limits on the slice.

These are plain systemd settings; the functional tests do not exercise
them.

## Changing the unit templates {#templates}

The templates in `/usr/lib/systemd/system` are the package's and are
replaced on an update. Change them with drop-ins (`systemctl edit`),
which are yours and stay.

- Adding restrictions is safe to try: `MemoryMax=`, `IPAddressDeny=` for
  the supervisor, `ProtectKernelTunables=`.
- **Do not** remove `PrivateNetwork=`, `PrivatePIDs=`, `PrivateIPC=`,
  `NoNewPrivileges=`, the empty `CapabilityBoundingSet=` or the
  `TemporaryFileSystem=` lines: each is a wall of
  [chapter 5](05-security.md#workload).
- Do not set `SELinuxContext=` in a drop-in of your own for the
  supervisor's or the workload's unit: the helper's drop-in
  `50-selinux.conf` sets it, with the sandbox's categories.
- The paths of the programs, of the state directory and of the mount
  points are fixed. The policy module labels them by path.

## Restarts {#restarts}

| What restarts | Running sandboxes |
|---|---|
| the driver | go on untouched; the gateway sees them again when the driver is back |
| the gateway | are stopped and started again by the gateway: workspaces are kept, processes are not |
| the helper | go on untouched; while it is down the driver can neither start nor stop one, and reports the helper's socket as the error |
| a workload's main process ends | the sandbox's phase becomes `Error`; nothing restarts it |

The driver reads the units' state every two seconds, which is how soon
the gateway learns that a sandbox failed.

## Reboot {#reboot}

A sandbox's units are not enabled, so nothing starts them at boot. The
sandbox's directory, with its workspace and its categories, is on disk
and stays. When the gateway is up again it starts the sandboxes it
wants running, each with a new launch, and the helper writes the
drop-ins in `/run` again.

This sequence is not covered by a functional test.

## Disk space {#disk}

Workspaces and launch files are in `/var/lib/openshell/systemd`.
Workspaces have no quota. Put `/var/lib/openshell` on a file system of
its own if a full workspace must not fill `/var`, and watch it as any
data directory:

```shell
du -sh /var/lib/openshell/systemd/*/workspace
```

A sandbox's directory is removed by deleting the sandbox through the
gateway. Only if the gateway no longer knows a sandbox whose directory
is still there: stop its units and remove the directory as root.

```shell
systemctl stop 'openshell-sandbox-*@<ID>.service'
rm -rf /var/lib/openshell/systemd/<ID> /run/systemd/system/openshell-sandbox-*@<ID>.service.d
systemctl daemon-reload
```

## Backups {#backup}

What is worth keeping is each `workspace/`. Launch files are
credentials for one launch and are written anew; do not restore them.
Restore a workspace into a stopped sandbox, with owners and SELinux
labels as they were (`tar --xattrs --selinux`, `rsync -aAX`): the files
must be `openshell-sandbox`'s and carry the sandbox's level. Restoring
has not been tested.

The driver's own configuration is the two drop-ins of
[chapter 2](02-installation.md) and the gateway's `gateway.toml`.

## What is recorded {#audit}

- The helper logs each start with the user it serves, whether sandboxes
  get categories (`selinux_categories`) and the state of its fence
  (`landlock`), and each connection it refuses from another user
  (`Rejected a helper client that is not the allowed user`).
- The driver logs each sandbox it creates, with ID and name.
- systemd logs each start and stop of a sandbox's units.
- With SELinux, the audit log has every denial of the four domains,
  with the sandbox's categories in the source context.

Who asked the gateway for a sandbox, and what a workload was refused on
the network, are OpenShell's records (the gateway's and the
supervisor's), not this driver's.
