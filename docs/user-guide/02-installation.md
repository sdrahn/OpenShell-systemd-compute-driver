---
title: "2. Installing and setting up"
description: Requirements of the OpenShell systemd compute driver, its packages on SLES 16 and openSUSE Leap 16, the three setup steps (helper, driver, gateway.toml), how to check the installation, and upgrading and removing it.
tags: [installation, setup, packages, administrator]
---

# 2. Installing and setting up

The driver is installed from packages, then connected in three steps:
the helper is told which user to serve, the driver is started as that
user, and the gateway is pointed at the driver's socket.

## Requirements {#requirements}

| Requirement | Why |
|---|---|
| SLES 16 or openSUSE Leap 16, x86_64 or aarch64 | what the packages are built for; tested on Leap 16.0 only |
| systemd 257 or later | `PrivatePIDs=`, the workload's own process ID namespace; the package requires it |
| a kernel with Landlock, Linux 6.2 or later | the helper's and the supervisors' fences; by default neither starts without it ([chapter 5](05-security.md#landlock)) |
| SELinux with the targeted policy, enforcing | not required, but several protections between sandboxes rest on it ([chapter 5](05-security.md#without-selinux)) |
| an OpenShell gateway of the release in [chapter 8](08-reference.md#packages), on the same host | the driver is built against that one release |
| the gateway runs as an ordinary user, as a service of that user (`openshell-gateway.service`) | the driver is a service of the same user and starts before it |

The gateway's user needs a user manager that runs without a login:
`loginctl enable-linger <user>`. OpenShell's own installer sets the
gateway up this way.

The helper gives each sandbox a user ID from 70000 to 79999. If the
host or its directory service (LDAP, Active Directory) has accounts in
that range, set another before the first sandbox is made:
`OPENSHELL_SYSTEMD_UID_RANGE` ([chapter 8](08-reference.md#helper)).

The tools a workload is to use are simply installed on the host
(`zypper in git-core gcc …`): a sandbox has no file system of its own.

## The packages {#packages}

| Package | Holds |
|---|---|
| `openshell-driver-systemd` | the driver, the helper, the supervisor's launcher, the unit templates, the user `openshell-sandbox`, and `openshell-supervisor` and `openshell-sandbox` of the OpenShell release it was built against |
| `openshell-driver-systemd-selinux` | the policy module `openshell_systemd`; installed with the first where `selinux-policy-targeted` is installed |
| `openshell-driver-systemd-docs` | this guide in Markdown, in `/usr/share/openshell-driver-systemd/docs`, with a collection file for mcp-docs; installed with the first where `mcp-docs` is installed |
| `openshell-driver-systemd-tests` | the functional tests, for a test system ([chapter 10](10-tested.md#run)); not for a production host |

```shell
zypper install openshell-driver-systemd
```

The package brings the two stock programs itself because an OpenShell
package does not install them on a host: the container drivers take
them from an image.

Installing creates the user `openshell-sandbox` (the one the unit
templates name; sandboxes run as users of their own), the state directory
`/var/lib/openshell/systemd` and the mount points the units bind a
sandbox's files to ([chapter 8](08-reference.md#files)). Nothing is
started.

## Step 1: tell the helper whom to serve {#helper}

The helper serves exactly one user and has no default for it. With
`<user>` the user the gateway runs as:

```shell
systemctl edit openshell-systemd-helper.service
```

```ini
[Service]
Environment=OPENSHELL_SYSTEMD_HELPER_ALLOW_USER=<user>
```

```shell
systemctl enable --now openshell-systemd-helper.service
```

Whoever is named here can make root create, start, stop and delete
sandboxes, and nothing else ([chapter 9](09-helper-protocol.md)). Name
the gateway's user and no person's account.

## Step 2: start the driver as the gateway's user {#driver}

As `<user>` (for a user without a login session:
`runuser -l <user> -c 'XDG_RUNTIME_DIR=/run/user/$(id -u) systemctl --user …'`):

```shell
systemctl --user edit openshell-driver-systemd.service
```

```ini
[Service]
Environment=OPENSHELL_SYSTEMD_TLS_CA=<path of the gateway's ca.crt>
```

```shell
systemctl --user enable --now openshell-driver-systemd.service
```

`OPENSHELL_SYSTEMD_TLS_CA` is the certificate authority a sandbox's
supervisor verifies the gateway with. A gateway set up by OpenShell's
installer keeps a `ca.crt` below its user's `~/.local` or `~/.config`.
The file must be readable by the gateway's user; the driver reads it at
every launch and hands its content on. Without the setting, supervisors
dial the gateway without TLS (`http://127.0.0.1:17670`): right only for
a gateway that serves plain HTTP on loopback.

A gateway on another port or address:
`OPENSHELL_GATEWAY_PORT` or `OPENSHELL_GRPC_ENDPOINT`
([chapter 8](08-reference.md#driver)).

The driver's socket is then
`/run/user/<uid>/openshell/systemd-driver.sock`, mode 0600.

## Step 3: point the gateway at the driver {#gateway}

In the gateway's `gateway.toml` (for a gateway that is a user's service:
`~/.config/openshell/gateway.toml`):

```toml
[openshell.gateway]
compute_driver = "systemd"

[openshell.drivers.systemd]
socket_path = "/run/user/<uid>/openshell/systemd-driver.sock"
```

Then restart the gateway:

```shell
systemctl --user restart openshell-gateway.service
```

Sandboxes that another driver made stay that driver's: delete them
before switching, or switch on a gateway without sandboxes.

## Checking the installation {#check}

```shell
systemctl is-active openshell-systemd-helper.service        # active
journalctl -u openshell-systemd-helper.service -n 5         # "Starting systemd helper … landlock=Enforced"
/usr/libexec/openshell/openshell-systemd-helper --allow-user <user> --self-test
openshell gateway info                                      # names openshell-driver-systemd and its version
openshell sandbox create --name first --detach -- sleep 600
openshell sandbox get first                                 # Phase: Ready
openshell sandbox exec first -- id -un                      # osb-70000
openshell sandbox delete first
```

The self-test enters the helper's fence, tries what the fence must
refuse and what it must allow, and prints `ok` or `FAIL` for each.

With SELinux, also:

```shell
semodule -l | grep -x openshell_systemd
ls -Z /usr/libexec/openshell/openshell-sandbox              # …:openshell_sandbox_exec_t:…
```

If one of these fails, see [chapter 7](07-troubleshooting.md).

## Upgrading {#upgrade}

Upgrade the packages, then restart the driver as the gateway's user
(`systemctl --user restart openshell-driver-systemd.service`). The
package restarts the helper itself, the way the distribution's packages
restart a system service. Neither restart touches a running sandbox.

- A running sandbox goes on with the programs and unit templates it was
  started with. It gets the new ones at its next launch (stop and
  start).
- The package does not change files in `/etc`: the drop-ins of steps 1
  and 2 and the gateway's configuration stay as they are.
- The driver and the gateway belong together: a driver package is built
  against one OpenShell release. Upgrade the gateway to the release the
  new package names, not past it.

An upgrade from one version of the package to the next has not been
tested yet, from one 0.1 version to another neither.

## Removing {#remove}

1. Delete the sandboxes through the gateway (`openshell sandbox delete`),
   or their workspaces stay in `/var/lib/openshell/systemd`.
2. Set `compute_driver` in `gateway.toml` back, remove the
   `[openshell.drivers.systemd]` table and restart the gateway.
3. Stop and disable the two services, then remove the packages:

   ```shell
   systemctl --user disable --now openshell-driver-systemd.service   # as the gateway's user
   systemctl disable --now openshell-systemd-helper.service
   zypper remove openshell-driver-systemd
   ```

Removing the package removes the policy module with it. It leaves what
it did not install as a file: the user `openshell-sandbox`,
`/var/lib/openshell/systemd`, the mount points (`/.openshell`,
`/sandbox`, two directories below `/etc/openshell`, `/etc/userdb` if it
was not there before), and the drop-ins of
steps 1 and 2. Remove them by hand if nothing else uses them.

## Without the package {#by-hand}

For development. `make build` builds the three programs; the two stock
programs come from an OpenShell build of the same release.

| What | Where |
|---|---|
| `openshell-driver-systemd` | `/usr/bin` |
| `openshell-systemd-helper`, `openshell-supervisor-fence`, `openshell-supervisor`, `openshell-sandbox` | `/usr/libexec/openshell/` |
| `systemd/*.service` | `/usr/lib/systemd/system/` |
| `systemd/user/*.service` | `/usr/lib/systemd/user/` |
| `systemd/tmpfiles.d/*.conf` | `/usr/lib/tmpfiles.d/`, then `systemd-tmpfiles --create` |
| the user | `useradd --system --user-group openshell-sandbox` |
| the policy module | `make selinux`, `semodule -i selinux/openshell_systemd.pp`, `restorecon -R /usr/libexec/openshell /usr/lib/systemd/system /var/lib/openshell /.openshell /sandbox` |

The paths are fixed: the unit templates and the policy name them.
