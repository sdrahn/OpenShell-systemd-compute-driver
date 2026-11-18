---
title: "7. Troubleshooting"
description: Symptoms, causes and fixes for the OpenShell systemd compute driver - the helper or the driver does not start, the gateway does not see the driver, a sandbox does not become ready or shows Error, something is refused inside a sandbox, SELinux and Landlock denials - and the error messages of the driver, the helper and the launcher.
tags: [troubleshooting, errors, administrator]
---

# 7. Troubleshooting

Start with the three places that say most: `openshell sandbox get NAME`,
the helper's journal, and the journal of the sandbox's units
([chapter 6](06-operation.md#logs)).

## The helper does not start {#helper}

`journalctl -u openshell-systemd-helper.service -n 20` shows the reason.

| Message | Cause and fix |
|---|---|
| `the following required arguments were not provided: --allow-user` | no user was named: [chapter 2, step 1](02-installation.md#helper) |
| `no user NAME` | the user of `OPENSHELL_SYSTEMD_HELPER_ALLOW_USER` does not exist |
| `invalid value … for '--uid-range <UID_RANGE>'` | `OPENSHELL_SYSTEMD_UID_RANGE` is not `FIRST-LAST`, starts below 1000, or touches 60001 to 65535, which the system keeps for itself |
| `openshell-systemd-helper must run as root` | it was started by hand as another user |
| `Landlock: file access: …` | the kernel has no Landlock, or one before Linux 6.2. Check `cat /sys/kernel/security/lsm`; add `lsm=landlock,…` to the kernel's command line if it is built in but off. As a last resort `OPENSHELL_SYSTEMD_HELPER_LANDLOCK=best-effort` ([chapter 5](05-security.md#landlock)) |
| `/usr/bin/stat: …` | SELinux is on and the label of `openshell-sandbox` could not be read. The helper then refuses to start, since it could not tell whether sandboxes need categories |
| `mount point /sandbox: missing, and its file system is read-only …` (or `/.openshell/…`) | a transactional system whose snapshot lacks the package's mount points, or `Read-only file system (os error 30)` from driver 0.1.4 or earlier, which made them each time the helper started | install the package again (`transactional-update pkg install -f openshell-driver-systemd`) and reboot into the new snapshot; with driver 0.1.4 or earlier, `transactional-update run mkdir -p /.openshell/supervisor /.openshell/channel /sandbox` and reboot |
| `restorecon: …` | relabelling the mount points failed; the message names the path |

It starts but logs `selinux_categories=false` on a host with SELinux:
the programs do not have the module's labels. Install
`openshell-driver-systemd-selinux`, or
`restorecon -R /usr/libexec/openshell`, and restart the helper.

## The driver does not start {#driver}

`journalctl --user -u openshell-driver-systemd.service -n 20`, as the
gateway's user.

| Message | Cause and fix |
|---|---|
| `systemd helper at /run/openshell-systemd-helper/helper.sock: No such file or directory` | the helper is not running |
| `systemd helper at …: Permission denied` | the helper serves another user: compare `OPENSHELL_SYSTEMD_HELPER_ALLOW_USER` with `id -un` |
| `systemd helper at …: unreadable answer: EOF while parsing …` | the helper closed the connection without an answer: it rejected the user (its journal says `Rejected a helper client that is not the allowed user`) |
| `the following required arguments were not provided: --bind-socket` | the driver was started by hand without its socket; use the unit |
| `Failed to connect to bus` when running `systemctl --user` | no user manager for the gateway's user: `loginctl enable-linger <user>`, and set `XDG_RUNTIME_DIR=/run/user/<uid>` |

## The gateway does not use the driver {#gateway}

`openshell gateway info` does not name `openshell-driver-systemd`, or
the gateway does not start.

- `compute_driver = "systemd"` and `[openshell.drivers.systemd]
  socket_path` in `gateway.toml`: is it the file the running gateway
  reads, and the socket of the right user ID?
- `ls -l /run/user/<uid>/openshell/systemd-driver.sock`: it must exist
  and be the gateway user's. The gateway and the driver must run as the
  same user; the socket serves no other.
- The gateway's own log (`journalctl --user -u openshell-gateway.service`)
  says what it made of the driver. A gateway of another release than
  the driver was built for may refuse it
  ([chapter 8](08-reference.md#packages)).

## A sandbox is refused at creation {#refused}

The message comes back from `openshell sandbox create`:

| Message | Meaning |
|---|---|
| `… takes no image (got "…")` | [chapter 3](03-sandboxes.md#no-image) |
| `… gives every sandbox a user of its own and cannot run one as …` | the policy or template asks for a particular user or group |
| `reserve a user for sandbox ID: no free user ID from FIRST to LAST` | every ID of the helper's range is a sandbox's or an account's. Delete sandboxes, or widen `OPENSHELL_SYSTEMD_UID_RANGE` |
| `the sandbox ID is not a canonical UUID` | the gateway sent an ID this driver does not accept; not expected from a stock gateway |
| `read the gateway CA PATH: …` | the file of `OPENSHELL_SYSTEMD_TLS_CA` is missing or unreadable for the gateway's user |
| `provision sandbox ID: environment variable "NAME" is not one a supervisor may be given` | a launch carried a variable that does not begin with `OPENSHELL_`, or one of this driver's; not expected from a stock gateway |
| `provision sandbox ID: no free pair of categories` | not expected below several hundred thousand sandboxes |
| `provision sandbox ID: chcon: …` or `restorecon: …` | labelling the sandbox's files failed; the helper's journal and the audit log say why |

## A sandbox does not become ready {#not-ready}

`openshell sandbox get NAME` stays at a phase before `Ready`, or the
create command returns an error that ends in the output of
`systemctl status` for the three units.

1. Which unit failed?

   ```shell
   systemctl status 'openshell-sandbox-*@<ID>.service'
   journalctl -u 'openshell-sandbox-*@<ID>.service' -n 50
   ```

2. By unit:

| Unit and symptom | Cause and fix |
|---|---|
| workload: `Unknown key name 'PrivatePIDs'` in the journal | systemd older than 257. It ignores the setting, and the workload would see the host's processes: do not run sandboxes there. The package requires 257 |
| supervisor: `openshell-supervisor-fence: Landlock: …` | as for the helper: the kernel's Landlock |
| supervisor: status `203/EXEC` or `Permission denied` on a program | wrong SELinux label after a manual installation: `restorecon -R /usr/libexec/openshell` |
| supervisor runs, logs that it cannot reach the gateway, or a certificate error | `OPENSHELL_SYSTEMD_TLS_CA` names the wrong CA or none; `OPENSHELL_GATEWAY_PORT` or `OPENSHELL_GRPC_ENDPOINT` do not match the gateway. The supervisor is on the host's network and dials `127.0.0.1` by default |
| any: `Failed to set up mount namespacing: /.openshell/…: No such file or directory` | a mount point is missing: `systemd-tmpfiles --create /usr/lib/tmpfiles.d/openshell-driver-systemd.conf` |
| any: `Failed at step USER` or `Permission denied` on the sandbox's own files | the unit was started without the helper's drop-in `40-user.conf`, by hand: start sandboxes through the gateway. Or the user `openshell-sandbox`, which the templates name, is missing: `systemd-sysusers` |

   The systemd messages are the usual ones for these causes; not each
   was provoked in a test.

3. With SELinux, look for denials of the sandbox's start:
   `ausearch -m avc -ts recent | grep openshell_`. A denial here on an
   unchanged Leap 16.0 or SLES 16 is a defect of the module; please
   report it with the denial's line.

## A sandbox shows `Error` {#error}

The workload's main process ended or was killed, or its supervisor
stopped (the workload is bound to it). The reason is in the two units'
journals. The memory limit of a drop-in shows as `oom-kill` in
`systemctl status`. Stop and start the sandbox, or delete it.

## Something is refused inside a sandbox {#inside}

| Symptom | Cause |
|---|---|
| `Read-only file system` | writing outside `/sandbox` and `/tmp` ([chapter 3](03-sandboxes.md#files)) |
| `Permission denied` on a path the policy lists | file modes for the sandbox's user, or a security file under SELinux; the policy cannot widen either |
| `id -un` or `whoami` cannot find a name for the user | the host's `nsswitch.conf` has no `systemd` for `passwd`. The sandbox works; its user has only a number |
| `Permission denied` on a path the policy does not list | OpenShell's Landlock rules for the workload; nothing is logged |
| a connection is refused or times out | the policy does not name the host, the port, or the **program** ([chapter 4](04-network-policy.md#binaries)) |
| a name does not resolve | the sandbox's name service answers by the policy |
| a TLS error in a program that has its own CA store | the supervisor ends TLS for `protocol: rest` endpoints with its own authority; the program must use the system's trust store that the sandbox is given |
| `Failed to connect to bus`, `systemctl` and `journalctl` fail | by design ([chapter 3](03-sandboxes.md#cannot)) |
| a tool is missing | it is not installed on the host |
| `fork: Resource temporarily unavailable` | `TasksMax=` ([chapter 6](06-operation.md#limits)) |

## A sandbox does not stop or cannot be deleted {#stuck}

A stop waits for systemd, which waits for the workload to end and kills
it after the unit's stop timeout (systemd's default, 90 seconds). The
helper serves one request at a time, so other requests wait that long.
`systemctl kill openshell-sandbox-workload@<ID>.service` ends it at
once.

`remove sandbox ID: …` in an error means the helper could not remove
the sandbox's directory; the message names the file.

## Finding what SELinux denied {#denials}

```shell
grep 'avc:  denied' /var/log/audit/audit.log | grep openshell_
```

The source context names the domain and the sandbox's categories
(`…:openshell_sandbox_t:s0:c7,c512`). Expected denials: a workload that
tries D-Bus, the journal's socket or another sandbox's files. To see
what a workload needs that the policy does not log, the tests' script
has `--selinux-discovery` ([chapter 10](10-tested.md#run)); never on a
production host.

`ausearch -ts boot` returned nothing on the Leap 16.0 test systems;
read the log file if `ausearch` is silent.

## Error messages {#messages}

From the driver, as the gateway reports them:

| Message | Meaning |
|---|---|
| `systemd helper at SOCKET: ERROR` | the helper could not be asked ([above](#driver)) |
| `systemd helper: unexpected answer …` | the helper and the driver are of different versions: restart both after an upgrade |
| `the sandbox has no launch authentication`, `decode systemd sandbox launch authentication: …`, `starting stopped sandbox units needs a launch authentication`, `starting a sandbox needs its specification for the new launch`, `the sandbox specification is for another sandbox` | the gateway's request lacks what a launch needs: a gateway of another release than the driver was built for |
| `the systemd compute driver does not support sandbox suspend`, `… retains no suspended sandbox state`, `… does not authenticate sandbox credentials` | not supported ([chapter 1](01-overview.md#not-supported)) |

From the helper, inside the driver's error:

| Message | Meaning |
|---|---|
| `the sandbox ID is not a canonical UUID` | a request with another ID; refused before anything is touched |
| `the request did not arrive in time` | a client connected and sent nothing for 30 seconds |
| `unreadable request: …`, `read the request: …` | not a request of this protocol, or larger than 4 MiB |
| `provision sandbox ID: …` | writing the launch files failed; the rest names the file or the program |
| `reserve a user for sandbox ID: …`, `write the user's name for sandbox ID: …`, `sandbox ID has no user` | the sandbox's user could not be chosen, named in `/run/userdb`, or read from its `uid` file |
| `systemctl start …: …` followed by the units' status | a unit did not start ([above](#not-ready)) |
| `remove sandbox ID: …`, `list sandboxes: …` | the state directory could not be read or changed |

Warnings in the helper's journal:

| Message | Meaning |
|---|---|
| `Left out of the Landlock fence` with a path | a path of the fence does not exist or could not be opened; the fence is tighter by it. Harmless unless something fails later for want of that path |
| `The helper runs without a Landlock fence` | `--landlock off`, or `best-effort` on a kernel without Landlock |
| `Rejected a helper client that is not the allowed user` with `peer_uid` | someone else connected to the socket |
