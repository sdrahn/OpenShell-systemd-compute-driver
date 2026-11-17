# OpenShell systemd compute driver

`openshell-driver-systemd` is an external compute driver for
[OpenShell](https://github.com/NVIDIA/OpenShell): a project of its own that a
stock OpenShell gateway uses through its extension driver interface. It runs
each sandbox as systemd units on the host of the OpenShell gateway, without
a container runtime or a VM. "The gateway" below is always the OpenShell
gateway. It uses the stock
`openshell-supervisor` and `openshell-sandbox` unchanged: policy, Landlock,
seccomp and network mediation are theirs, as with every other driver.

This driver is for Linux hosts where the workload should use the host's own
installed tools, and where systemd units and the host's security policy are
how services are run and confined.

## What a sandbox is

Three instances of unit templates, named by the sandbox's ID:

| Unit | Runs | As | Network |
|---|---|---|---|
| `openshell-sandbox-netns@ID.service` | holds the sandbox's network namespace | root | a namespace with loopback only |
| `openshell-sandbox-supervisor@ID.service` | `openshell-supervisor` | `openshell-sandbox` | the host's |
| `openshell-sandbox-workload@ID.service` | `openshell-sandbox`, which starts the agent | `openshell-sandbox` | the namespace of the unit above |

The outer fence is the network namespace: the workload has no interface but
loopback, so all egress goes through the supervisor's control socket. Both
units have no capabilities and `NoNewPrivileges=yes`. The workload also has
a process ID namespace of its own (`PrivatePIDs=`, systemd 257 or later):
every sandbox and supervisor runs as one user, and without it an agent could
signal the processes of another sandbox.

The workload's root file system is the host's, read-only. A sandbox of this
driver has no image; a request that names one is refused.

## Two processes

The gateway's driver socket accepts only the gateway's own user, so the
driver runs as that user and cannot start system units. The crate therefore
builds two programs:

- `openshell-driver-systemd` is the `ComputeDriver` service. It builds a
  launch's bootstrap file, runtime descriptor and TLS material, as the
  container drivers do, and has no privilege.
- `openshell-systemd-helper` runs as root. It writes a launch's files to a
  directory of its own and starts, stops and reads the state of the three
  units. It serves one user, checked from its socket's peer credentials.

The helper is the privilege boundary. A request carries file contents and a
sandbox ID, never a path, a unit name or a command. The ID must be a
canonical UUID and is the only part of a request that reaches a path or a
unit name. The units themselves are the templates in `systemd/`, installed
with the package, which an administrator can read and override with
drop-ins.

The protocol is one JSON request and one JSON response per connection
(`src/protocol.rs`).

## Files of a sandbox

`/var/lib/openshell/systemd/ID/`, written by the helper:

| Path | Owner, mode | Mounted in |
|---|---|---|
| `channel/sandbox/` (bootstrap file, TLS identity) | workload user, 0711 | both, read-only for the supervisor |
| `control/` (the control socket) | root, 1777 | both |
| `supervisor/` (runtime descriptor, auth bundle) | workload user, 0700 | the supervisor only |
| `etc/` (gateway token, gateway CA) | workload user, 0700 | the supervisor only |
| `supervisor.env` | root, 0600 | read by systemd |
| `workspace/` | workload user, 0700 | the workload, at `/sandbox` |

The supervisor and the workload run as the same user, as in the Podman
driver. What keeps the supervisor's credentials from the agent is the
workload's mount namespace, which does not contain them, and Landlock: the
workload's unit mounts an empty file system over `/var/lib/openshell` and
over `/run/openshell-sandbox`, where each supervisor's sockets and proxy
keys are, and binds in the channel and the workspace only.

## Install

On SLES 16 and openSUSE Leap 16, install the package
`openshell-driver-systemd` (`packaging/suse`). It holds the two programs,
the unit templates, the user `openshell-sandbox`, and `openshell-supervisor`
and `openshell-sandbox` of the OpenShell release it was built against: an
OpenShell package does not install those two on a host. Where SELinux's
targeted policy is installed, `openshell-driver-systemd-selinux` comes with
it.

Then, with `<user>` the user the OpenShell gateway runs as:

1. Name that user for the helper in a drop-in and start the helper:

   ```shell
   systemctl edit openshell-systemd-helper.service   # [Service] Environment=OPENSHELL_SYSTEMD_HELPER_ALLOW_USER=<user>
   systemctl enable --now openshell-systemd-helper.service
   ```

2. As `<user>`, give the driver the certificate authority supervisors verify
   the gateway with, and start it:

   ```shell
   systemctl --user edit openshell-driver-systemd.service   # [Service] Environment=OPENSHELL_SYSTEMD_TLS_CA=<the gateway's ca.crt>
   systemctl --user enable --now openshell-driver-systemd.service
   ```

3. Point the gateway at the driver's socket in its `gateway.toml` and
   restart it:

   ```toml
   [openshell.gateway]
   compute_driver = "systemd"

   [openshell.drivers.systemd]
   socket_path = "/run/user/<uid>/openshell/systemd-driver.sock"
   ```

`openshell gateway info` then names `openshell-driver-systemd` and its
version.

The package's `tmpfiles.d` entry creates the helper's state directory and
the mount points the units bind to (`/.openshell`, `/sandbox`, two
directories in `/etc/openshell`), which are the paths the stock programs use
in a container.

## Build

`make build` builds the two programs, `make check` runs the format check,
clippy and the unit tests, `make selinux` builds the policy module.
OpenShell does not publish its crates, so Cargo fetches them from the
release tag named in `Cargo.toml`. The driver uses crates OpenShell does not
promise to keep stable: it is built and tested against that one release.

Without the package: put `openshell-driver-systemd` in `/usr/bin`, the
helper and the two stock programs in `/usr/libexec/openshell/`, the units of
`systemd/` in `/usr/lib/systemd/system/` and of `systemd/user/` in
`/usr/lib/systemd/user/`, `systemd/tmpfiles.d/` in `/usr/lib/tmpfiles.d/`
(then `systemd-tmpfiles --create`), and create the user with
`useradd --system --user-group openshell-sandbox`.

## SELinux

`selinux/` holds an optional policy module with a domain for each of the two
sandbox units, one for the helper and one for the unit of a sandbox's network
namespace. The units enter the domains through the labels of the two
programs, so they need no change, and they start as before on a host
without the module.

- `openshell_sandbox_t`, the workload: `openshell-sandbox` and everything it
  starts. It reads the host as far as file modes let its user, apart from the
  files the policy counts as security files. It writes its workspace, its
  temporary files and its own directory under `/run`. It reaches its
  supervisor's control socket and no other service's socket: not D-Bus, not
  the journal. It cannot signal or trace another domain.
- `openshell_supervisor_t`, the supervisor: its launch files, its runtime
  directory, the control socket, and the network.
- `openshell_systemd_helper_t`, the helper: the sandboxes' files, and
  starting, stopping and asking about the sandbox units and no others. It
  has no network. It still runs as root and writes drop-ins among systemd's
  runtime units, so the domain bounds a mistake in the helper; it does not
  make the helper harmless.
- `openshell_sandbox_netns_t`, the unit that holds a sandbox's network
  namespace: one network setting of that namespace, written as root, and
  nothing else.

The package loads the module. By hand, with the policy development files of
the distribution (`selinux-policy-devel`):

```shell
make selinux
semodule -i selinux/openshell_systemd.pp
restorecon -R /usr/libexec/openshell /var/lib/openshell /.openshell /sandbox
systemctl restart openshell-systemd-helper.service
```

The helper runs `restorecon` on the files it writes for a launch, because a
new file takes its directory's label and the domains tell a sandbox's files
apart by label. A workspace's own files keep the labels they were made with.

Every sandbox also gets a pair of categories of its own, as a container
does. The helper chooses the pair, keeps it for the sandbox's life, gives
the sandbox's files that level and writes drop-ins in `/run/systemd/system`
that start its two units with it. Two sandboxes run in the same domains and
as the same user; with the categories the policy refuses one what is the
other's, also for a workload that has left its namespaces.

The driver has no domain of its own. It runs in the session of the gateway's
user, next to the gateway and as the same user, and holds nothing the gateway
does not; its unit is held to what a user's service can be held to without
privileges (no new privileges, no namespaces, three kinds of socket).

The module was written against the policy of openSUSE Leap 16.0.

## Landlock

The helper runs as root, and before it serves a request it puts a Landlock
fence around itself, which binds root too. Afterwards it reaches its state
directory, the directory of the drop-ins and its socket, reads `/usr`,
`/etc`, `/proc`, `/sys` and `/run`, and takes no TCP port and makes no TCP
connection. The programs it runs (`systemctl`, `restorecon`, `chcon`,
`stat`) are behind the same fence. This is a second layer next to the
SELinux domain, and the only one on a host without the module.

`--landlock` (`OPENSHELL_SYSTEMD_HELPER_LANDLOCK`) says what to do on a
kernel that cannot hold the helper to its files (Landlock before Linux 6.2,
or none): `required`, the default, does not start; `best-effort` goes on
with what the kernel offers; `off` asks for none.
`openshell-systemd-helper --allow-user <user> --self-test` enters the fence,
tries what it must refuse and what it must allow, and prints each result.

The fence does not cover everything: it grants a directory, not names in
it, so the helper could write a drop-in for any unit; and Landlock does not
mediate connecting to a Unix socket by its path, nor changes of a file's
owner, mode or label.

The supervisor is the stock program, and every sandbox's supervisor is the
same user: file modes would let one read another sandbox's credentials. Its
unit starts it through `openshell-supervisor-fence`, which enters a fence
that names this sandbox's files (its launch files, its runtime directory,
the control socket's directory, its workspace to read) besides `/usr`,
`/etc`, `/proc` and `/sys`, and then becomes the supervisor. The network is
not part of this fence: what a sandbox may reach is OpenShell's policy,
which the supervisor is there to hold. `--landlock`
(`OPENSHELL_SYSTEMD_SUPERVISOR_LANDLOCK`, in a drop-in of the unit) has the
same three values as the helper's.

The workload is behind a Landlock fence of OpenShell's own, from the
sandbox's policy: `openshell-sandbox` sets it.

## Not supported

- Images: there is no root file system but the host's.
- CPU, memory and GPU requests. `TasksMax=` in the workload's template is
  the only limit; set others with drop-ins.
- Suspend and resume, and sandbox authentication.
- A workload user per sandbox: every sandbox runs as `openshell-sandbox`. A
  policy that asks for another user or group is refused.
