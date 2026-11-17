---
title: "5. The security model"
description: What the OpenShell systemd compute driver protects against and with what - the threat model, the layers around a workload, a supervisor and the helper, the SELinux module and its categories, the Landlock fences and their setting, what keeps sandboxes apart, what is weaker without SELinux, and the known limits.
tags: [security, selinux, landlock, administrator]
---

# 5. The security model

A sandbox of this driver is a set of processes of one unprivileged host
user, fenced in by namespaces, systemd settings, Landlock, seccomp and
SELinux. This chapter says what each layer holds and where the limits
are. The full review before the first release, with its findings, is
[security-review-0.1.md](../security-review-0.1.md).

## What it is built against {#threats}

1. A workload that is hostile or was taken over (an agent that was
   prompt-injected, a tool's malicious output), inside its sandbox.
2. One sandbox against another, and against the supervisors.
3. A local user without privileges, against the helper and the driver.
4. The gateway's user: what that user can make root do beyond what the
   helper is for.
5. A supervisor that was taken over.

Not in it: root on the host, and the gateway itself. Whoever controls
the gateway controls every sandbox's policy.

## Compared with a container or a VM {#compared}

A sandbox here shares the host's kernel, the host's file system
(read-only) and one user with every other sandbox. That is less
separation than a VM gives and, in what a workload can **read**, less
than a container gives. Choose the driver when reading the host is
wanted, and not for workloads that must learn nothing about it.

## The layers around a workload {#workload}

| Layer | Set by | Holds |
|---|---|---|
| network namespace, loopback only | the unit (`PrivateNetwork=`, joined from the namespace unit) | no connection but through the supervisor |
| process ID namespace | the unit (`PrivatePIDs=`) | sees and signals only its own processes |
| IPC namespace | the unit (`PrivateIPC=`) | no shared memory or message queues of the host or another sandbox |
| mount namespace | the unit | host read-only (`ProtectSystem=strict`); no `/home`, `/root`; empty `/run` and `/var/lib/openshell`; of the sandbox's files only the channel and the workspace |
| no capabilities, `NoNewPrivileges=yes` | the unit | no privilege, and no way to one |
| `TasksMax=2048` | the unit | a bound on processes and threads |
| Landlock | `openshell-sandbox`, from the sandbox's policy | the policy's paths only |
| seccomp | `openshell-sandbox` | the system calls OpenShell's filter refuses |
| SELinux domain `openshell_sandbox_t` | the policy module | no socket of a host service, no security files, no signal to another domain; only its own categories' files |

The supervisor's credentials (the gateway token, the authentication
bundle, the proxy's keys) are not in the workload's mount namespace,
and with SELinux their type is a security file that the workload's
domain may not read.

## The layers around a supervisor {#supervisor}

The supervisor is on the host's network and holds its sandbox's
credentials. It is trusted more than a workload, but every sandbox's
supervisor is the same user.

| Layer | Holds |
|---|---|
| user `openshell-sandbox`, no capabilities, `NoNewPrivileges=yes` | no privilege |
| mount namespace: empty `/var/lib/openshell` and `/run/openshell-sandbox` but for its own sandbox's files and runtime directory | another sandbox's files and sockets are not there |
| Landlock fence, entered by `openshell-supervisor-fence` | `/usr` to read and run; `/etc`, `/proc`, `/sys` to read; its own launch files, runtime directory, control socket directory and `/tmp` to write |
| SELinux domain `openshell_supervisor_t` with the sandbox's categories | its own sandbox's files and processes |
| its environment | only variables that begin with `OPENSHELL_` and not with `OPENSHELL_SYSTEMD_`: a launch cannot set `LD_PRELOAD`, `PATH` or a setting of this driver |

The network is not part of the supervisor's fence. What a sandbox may
reach is OpenShell's policy, and the supervisor is the program that
holds it.

## The layers around the helper {#helper}

The helper runs as root. What bounds it:

| Layer | Holds |
|---|---|
| its socket: mode 0600, the allowed user's; the peer's user checked for every connection | nobody else is served, root included |
| the protocol | a request carries file contents and a sandbox ID, never a path, a unit name or a command; the ID must be a canonical UUID ([chapter 9](09-helper-protocol.md)) |
| Landlock fence, entered before the first request | writes only its state directory, the directory of the drop-ins and its socket's directory; reads `/usr`, `/etc`, `/proc`, `/sys`, `/run`; no TCP |
| five capabilities (`CAP_CHOWN`, `CAP_DAC_OVERRIDE`, `CAP_DAC_READ_SEARCH`, `CAP_FOWNER`, `CAP_FSETID`) | of root's rights, those for files of another user |
| `ProtectHome=yes`, Unix and netlink sockets only | no home directories, no network |
| SELinux domain `openshell_systemd_helper_t` | the sandboxes' files; starting, stopping and asking about the sandbox units and no others |

The driver has no privilege and no SELinux domain of its own. It runs
next to the gateway as the same user and holds nothing the gateway does
not. Its socket serves that user only; it has no TCP listener.

## What keeps sandboxes apart {#between-sandboxes}

Every workload and supervisor is the user `openshell-sandbox`, so file
modes alone separate nothing.

| Between | With the SELinux module | Without it |
|---|---|---|
| workload and another sandbox's files | not in its mount namespace; refused by categories even outside the namespace | not in its mount namespace |
| workload and another sandbox's or the host's processes | process ID namespace; domain | process ID namespace |
| workload and its own supervisor's credentials | not mounted; security file | not mounted |
| supervisor and another sandbox's files | not mounted; Landlock fence; categories | not mounted; Landlock fence |
| supervisor and another supervisor's processes | a real signal is refused by categories; signal 0 is not | **nothing**: one supervisor can signal another |

## The SELinux module {#selinux}

`openshell-driver-systemd-selinux` loads the module `openshell_systemd`.
The units enter its domains through the labels of their programs, so the
same unit files work on a host without it.

- **`openshell_sandbox_t`**, the workload. Reads the host as far as file
  modes let its user, apart from security files. Writes its workspace,
  its temporary files and its own directory under `/run`. Connects to
  its supervisor's control socket and no other service's: not D-Bus,
  not the journal.
- **`openshell_supervisor_t`**, the supervisor: its launch files, its
  runtime directory, the control socket, the network.
- **`openshell_systemd_helper_t`**, the helper.
- **`openshell_sandbox_netns_t`**, the unit that makes the network
  namespace: one network setting of that namespace, as root, and
  nothing else.

**Categories.** Each sandbox gets a pair of MCS categories of its own
(out of 1024, as containers do). The helper chooses the pair at the
first launch, keeps it for the sandbox's life, gives the sandbox's files
that level and writes drop-ins that start its two units with it
(`/run/systemd/system/openshell-sandbox-{supervisor,workload}@ID.service.d/50-selinux.conf`).
Two sandboxes then run in the same domains with different categories,
and the policy refuses one what is the other's.

The helper labels the files of a launch. Files a workload makes in its
workspace carry the label and level they were made with.

The module was written against the policy of openSUSE Leap 16.0.

## The Landlock fences {#landlock}

Landlock is the kernel's way for a program to give up access for good.
It binds root too. Two programs of this driver use it on themselves;
the third fence, the workload's, is OpenShell's.

| Program | Setting | Default |
|---|---|---|
| the helper | `--landlock`, `OPENSHELL_SYSTEMD_HELPER_LANDLOCK` | `required` |
| the supervisor's launcher | `--landlock` in the unit's `ExecStart=` only | `required` |

| Value | On a kernel that cannot hold the program to its files |
|---|---|
| `required` | the program does not start |
| `best-effort` | it goes on with what the kernel offers, down to nothing, and logs a warning |
| `off` | no fence is asked for |

"Cannot hold it to its files" means no Landlock or one older than Linux
6.2. The helper's ban on TCP needs Linux 6.7 and is left out silently
on an older kernel.

The launcher's setting is an argument and not a variable on purpose:
the supervisor's environment comes with a launch, and a launch must not
decide this. To change it, override `ExecStart=` of
`openshell-sandbox-supervisor@.service` in a drop-in.

What a fence does not cover: it grants a directory, not names in it; it
does not mediate connecting to a Unix socket by its path, signals, or
changes of a file's owner, mode or
label.

## What is weaker without SELinux {#without-selinux}

The units start and work on a host without the module. Then:

- nothing refuses a workload the sockets of host services but the fact
  that the unit's `/run` does not hold them;
- two sandboxes are kept apart by namespaces and the supervisors' fences
  only, with no second layer if a workload leaves its namespaces;
- one supervisor can signal, and so stop, another;
- the helper is bounded by its fence and capabilities only.

No functional test runs on such a host. Run the driver with SELinux
enforcing, as SLES 16 and Leap 16 are installed.

## Known limits {#limits}

- **One user for all sandboxes.** The root of most rows above. A user
  for every sandbox is planned (issue #21 of the project).
- **Signal 0 between supervisors**, also with SELinux: one supervisor
  can learn whether a process of another exists. The distribution's
  policy does not hold that signal to categories.
- **A helper that was taken over** can write a drop-in for any unit of
  the host: its fence and its domain grant the drop-in directory, not
  names in it. No field of a request reaches a drop-in's path or
  content.
- **No limits on memory, CPU or disk** per sandbox: a workload can fill
  the file system of `/var/lib/openshell` or exhaust memory, unless the
  administrator sets limits ([chapter 6](06-operation.md#limits)).
- **A workload reads the host**: every file an unprivileged user can
  read, configuration in `/etc` among it. Keep secrets out of
  world-readable files, as on any multi-user host.
- **Requests to the helper are served one at a time.** A stop that
  waits for a workload holds up the next request.
- **The build**: the package builds OpenShell's two programs with
  OpenShell's default features and its lock file; those crates are
  OpenShell's to check. There is no checksum of OpenShell's source
  archive in the repository.
- **No person has reviewed the code yet**; an agent wrote it and a
  second agent reviewed it.
