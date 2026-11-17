---
title: "3. Working in a sandbox"
description: What a sandbox of the OpenShell systemd compute driver is for the person or agent working in it - creating one, why an image is refused, which files it reads and writes, the workspace, which tools work, what is out of reach, stop, start and delete, and the differences from a container sandbox.
tags: [user, sandbox, workspace, agent]
---

# 3. Working in a sandbox

A sandbox of this driver is used like any other OpenShell sandbox, with
the `openshell` command or the gateway's API. What is different is what
is inside: the host itself, read-only, instead of an image.

## Creating and using one {#create}

```shell
openshell sandbox create --name work --detach -- sleep 3600
openshell sandbox get work                    # Phase: Ready, and its Id
openshell sandbox exec work -- git --version
openshell sandbox stop work
openshell sandbox start work
openshell sandbox delete work
```

A policy is given with `--policy FILE` ([chapter 4](04-network-policy.md)).
Everything else of the `openshell` command is OpenShell's and described
in its documentation.

## Why an image is refused {#no-image}

```text
the systemd compute driver runs workloads on the host's file system and takes no image (got "registry.example/image:1")
```

There is no root file system but the host's. Leave out `--from` (and an
image in a sandbox template). A tool the workload needs is installed on
the host by its administrator; a tool only this sandbox needs can be
put into its workspace.

For the same reason a policy that asks for a workload user or group
other than the sandboxes' own (`sandbox`, or its number) is refused:

```text
the systemd compute driver runs every workload as UID:GID and cannot run one as "USER":"GROUP"
```

## What a workload is {#identity}

| Property | Value |
|---|---|
| user and group | `openshell-sandbox`, never root; the same for every sandbox |
| capabilities | none; it cannot gain any (`NoNewPrivileges`) |
| home and working directory of a command | `/sandbox` (`HOME=/sandbox`) |
| processes it sees | its own sandbox's only |
| network interfaces | loopback |
| system calls | filtered by OpenShell's seccomp policy |

It cannot become root: `sudo` and other set-user-ID programs do not
work in it.

## Which files it reads and writes {#files}

Three things decide together, and the tightest wins:

1. **The host's file modes** for the user `openshell-sandbox`: it reads
   what any unprivileged user of the host reads.
2. **OpenShell's file system policy** of the sandbox, which
   `openshell-sandbox` sets as a Landlock rule set. OpenShell's default
   is read-only for `/bin`, `/usr`, `/lib`, `/proc`, `/dev/urandom`,
   `/app`, `/etc` and `/var/log`, and read-write for `/sandbox`, `/tmp`
   and `/dev/null`. A policy file can name others
   ([chapter 4](04-network-policy.md#filesystem)).
3. **The unit**: the whole host is mounted read-only but the paths
   below; `/home`, `/root` and `/run/user` are empty; `/run` and
   `/var/lib/openshell` are empty file systems of the sandbox's own.
   With SELinux, the workload's domain also refuses the host's security
   files (`/etc/shadow` among them).

| Path in the sandbox | What it is |
|---|---|
| `/sandbox` | the workspace: the sandbox's own, read-write, kept until the sandbox is deleted |
| `/tmp` | the sandbox's own, read-write; programs written there can be run. Emptied when the sandbox stops |
| `/usr`, `/etc`, `/proc`, … | the host's, read-only |
| `/etc/resolv.conf` | the sandbox's own: its name service is its supervisor's |
| `/run` | empty but for what the sandbox's own programs put there |

A policy cannot give more than the unit: a `read_write` entry for
`/etc` leaves `/etc` read-only.

## The workspace {#workspace}

`/sandbox` in the sandbox is
`/var/lib/openshell/systemd/<sandbox ID>/workspace` on the host: a
directory of the user `openshell-sandbox`, mode 0700.

- It is kept across stop and start, and across restarts of the driver,
  the helper and the gateway.
- `openshell sandbox delete` removes it with everything in it. There is
  no undo.
- It has no size limit of its own: it shares the file system of
  `/var/lib/openshell`.
- The host's administrator reads it as root. Another sandbox does not
  ([chapter 5](05-security.md#between-sandboxes)).

## What works in a sandbox {#works}

The functional tests run these in a sandbox with OpenShell's default
policy, with the tools installed on the host:

- git: a repository, commits, diffs, branches;
- a C compiler and `make`; running what was built;
- Python: scripts, processes in parallel, a virtual environment in the
  workspace;
- servers and clients on loopback, also on ports below 1024; Unix
  sockets in the workspace;
- `tar`, temporary files, scripts in `/tmp`;
- listing, signalling and waiting for its own processes;
- reading the host: `rpm -q`, `/etc/os-release`, `uname`, `ps`, `free`,
  `df`, `ip addr`, `ss`, `find` and `grep` over `/usr/share/doc`.

What `ps`, `free` and `df` show is the host's memory and disks, and the
sandbox's own processes.

## What is out of reach {#cannot}

| Not possible | Why |
|---|---|
| writing outside `/sandbox` and `/tmp` | the host is mounted read-only |
| `systemctl`, `journalctl`, `busctl`, anything over D-Bus | the sockets of the host's services are not in the sandbox's `/run`; the workload's SELinux domain refuses them too |
| a direct connection to the network | there is no interface; see [chapter 4](04-network-policy.md) |
| `mount`, `unshare`, making namespaces | no capabilities, and OpenShell's seccomp filter |
| seeing or signalling another sandbox's or the host's processes | the process ID namespace |
| reading the sandbox's own credentials for the gateway | they are its supervisor's and not mounted in the workload |
| a pseudo-terminal from `/dev/ptmx`, shared memory in `/dev/shm` | not among the paths of OpenShell's default policy |
| installing packages (`zypper install`) | needs root and a writable `/usr` |

So a workload can inspect this host, and cannot change it. An agent
that is to administer the host needs a way to do that which is outside
the sandbox and under someone's control; this driver provides none.

## Stop, start, delete {#lifecycle}

| Command | What happens |
|---|---|
| `stop` | the three units are stopped; the sandbox's files and workspace stay; `/tmp` is gone |
| `start` | a new launch: new launch files and a new supervisor and workload, with the same workspace and the same SELinux categories |
| `delete` | the units are stopped and the sandbox's directory is removed, the workspace too |

If the workload's main process ends or is killed, the sandbox's phase
becomes `Error`; the driver does not restart it. Stop and start it, or
delete it.

When the gateway restarts it stops its sandboxes and starts them again:
the workspace is kept, running processes are not. A restart of the
driver alone leaves a running sandbox untouched.

## Differences from a container sandbox {#differences}

| | Container drivers | This driver |
|---|---|---|
| root file system | an image | the host's, read-only |
| tools | what the image holds | what the host has installed |
| user in the sandbox | the image's sandbox account | `openshell-sandbox` of the host, shared by all sandboxes |
| writable | the container's file system | `/sandbox` and `/tmp` |
| CPU and memory limits of a sandbox | the runtime's | none per sandbox ([chapter 6](06-operation.md#limits)) |
| what a workload learns about the host | little | what an unprivileged user can read |
| network, policy, credentials | OpenShell's supervisor | the same supervisor, unchanged |

The last row is why policies written for a container sandbox work here
as far as they do not name an image, a user or paths that exist only in
an image.
