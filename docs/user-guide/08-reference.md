---
title: "8. Reference"
description: Reference of the OpenShell systemd compute driver - the programs with every argument and environment variable, the units and their settings, files and directories, sockets, the user, the SELinux types, the packages and the OpenShell release they are built against.
tags: [reference, settings, units, files, selinux]
---

# 8. Reference

Every setting, file and name of the driver. Settings of a service go
into a drop-in (`systemctl edit`, for the driver `systemctl --user
edit`) as `Environment=NAME=value`.

## openshell-driver-systemd, the driver {#driver}

`/usr/bin/openshell-driver-systemd`, started by the user unit
`openshell-driver-systemd.service`.

| Argument | Variable | Default | Meaning |
|---|---|---|---|
| `--bind-socket PATH` | `OPENSHELL_COMPUTE_DRIVER_SOCKET` | none; required. The unit passes `%t/openshell/systemd-driver.sock` | the socket the gateway reaches the driver on. Only the driver's own user is served. There is no TCP listener |
| `--gateway-tls-ca PATH` | `OPENSHELL_SYSTEMD_TLS_CA` | none | the CA supervisors verify the gateway with. Set: supervisors dial `https://`. Read at every launch |
| `--grpc-endpoint URL` | `OPENSHELL_GRPC_ENDPOINT` | `http://127.0.0.1:PORT`, or `https://…` with a CA | the gateway endpoint supervisors dial; they are on the host's network |
| `--gateway-port PORT` | `OPENSHELL_GATEWAY_PORT` | `17670` | the port of the default endpoint |
| `--gateway-name NAME` | `OPENSHELL_GATEWAY_NAME` | none | the gateway's name, for the supervisors' logs and telemetry |
| `--helper-socket PATH` | `OPENSHELL_SYSTEMD_HELPER_SOCKET` | `/run/openshell-systemd-helper/helper.sock` | the helper's socket |
| `--log-level LEVEL` | `OPENSHELL_LOG_LEVEL` | `info` | `error`, `warn`, `info`, `debug`, `trace` |
| `--admission-config-json JSON` | `OPENSHELL_DRIVER_ADMISSION_CONFIG_JSON` | `{}` | OpenShell's resource admission settings for a driver. This driver acknowledges them and applies no resource request |
| `--version` | | | prints the driver's version |

## openshell-systemd-helper, the helper {#helper}

`/usr/libexec/openshell/openshell-systemd-helper`, started by
`openshell-systemd-helper.service` as root.

| Argument | Variable | Default | Meaning |
|---|---|---|---|
| `--allow-user USER` | `OPENSHELL_SYSTEMD_HELPER_ALLOW_USER` | none; required | the only user served: the gateway's |
| `--landlock MODE` | `OPENSHELL_SYSTEMD_HELPER_LANDLOCK` | `required` | `required`, `best-effort` or `off` ([chapter 5](05-security.md#landlock)) |
| `--socket PATH` | `OPENSHELL_SYSTEMD_HELPER_SOCKET` | `/run/openshell-systemd-helper/helper.sock` | its socket; change it for the driver too |
| `--uid-range FIRST-LAST` | `OPENSHELL_SYSTEMD_UID_RANGE` | `70000-79999` | the user IDs sandboxes are given, one each; the group has the same number. From 1000 up, and outside 60001 to 65535. An ID that an account or group of the host has is left out. A sandbox keeps its ID when the range is changed |
| `--state-dir DIR` | | `/var/lib/openshell/systemd` | where sandboxes' files are kept. The unit templates and the policy module name the default: leave it |
| `--log-level LEVEL` | `OPENSHELL_LOG_LEVEL` | `info` | as for the driver |
| `--self-test` | | | enters the fence, tries what it must refuse and allow, prints `ok` or `FAIL` per line; exit status 0 if all are as they must be |
| `--version` | | | prints the version |

## openshell-supervisor-fence, the supervisor's launcher {#fence}

`/usr/libexec/openshell/openshell-supervisor-fence`, in the `ExecStart=`
of `openshell-sandbox-supervisor@.service`.

| Argument | Default | Meaning |
|---|---|---|
| `--runtime-dir DIR` | required | the supervisor's runtime directory, writable behind the fence |
| `--landlock MODE` | `required` | as the helper's. An argument only: it has no variable |
| `-- COMMAND…` | required | the program to become, behind the fence |

## The stock programs {#stock}

`/usr/bin/openshell-supervisor` and `/usr/bin/openshell-sandbox` are
OpenShell's, from the packages `openshell-supervisor` and
`openshell-sandbox` of the release below, which the driver's package
requires. The policy module labels them for the sandboxes' domains,
which they enter only when systemd starts them as a sandbox's units. Their arguments are in the unit
templates; their settings come with each launch from the gateway and
the driver.

## Units {#units}

| Unit | Kind | Notable settings |
|---|---|---|
| `openshell-systemd-helper.service` | system | root; `RuntimeDirectory=openshell-systemd-helper`; five capabilities; `ProtectHome=yes`; `RestrictAddressFamilies=AF_UNIX AF_NETLINK`; `Restart=on-failure` |
| `openshell-driver-systemd.service` | user | `Before=openshell-gateway.service`; `NoNewPrivileges=yes`; `RestrictNamespaces=yes`; `RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6`; `Restart=on-failure` |
| `openshell-sandbox-netns@.service` | system, template | one-shot, stays active; `PrivateNetwork=yes`, `PrivateTmp=yes`, `PrivateIPC=yes`; sets `net.ipv4.ip_unprivileged_port_start=0` in the namespace; `CapabilityBoundingSet=CAP_NET_ADMIN`; `StopWhenUnneeded=yes` |
| `openshell-sandbox-supervisor@.service` | system, template | `User=openshell-sandbox`; `EnvironmentFile=` the sandbox's `supervisor.env`; `RuntimeDirectory=openshell-sandbox/%i` (0700); empty `/var/lib/openshell` and `/run/openshell-sandbox` but for its own; no capabilities; `ProtectSystem=strict`, `ProtectHome=yes`, `PrivateTmp=yes`; `StopWhenUnneeded=yes` |
| `openshell-sandbox-workload@.service` | system, template | `User=openshell-sandbox`; `HOME=/sandbox`; requires and joins the namespaces of the netns unit; `BindsTo=` the supervisor's unit; `PrivateNetwork=`, `PrivatePIDs=`, `PrivateIPC=`, `PrivateTmp=`; empty `/run` and `/var/lib/openshell`; no capabilities; `ProtectSystem=strict`, `ProtectHome=yes`; `TasksMax=2048` |

The instance name (`%i`) is the sandbox's ID. None of the sandbox units
is enabled; the helper starts and stops them.

## Files and directories {#files}

| Path | What |
|---|---|
| `/usr/lib/systemd/system/openshell-*.service` | the system units |
| `/usr/lib/systemd/user/openshell-driver-systemd.service` | the driver's unit |
| `/usr/lib/sysusers.d/openshell-driver-systemd.conf` | the user `openshell-sandbox`, which the unit templates name |
| `/usr/lib/tmpfiles.d/openshell-driver-systemd.conf` | creates the state directory and the mount points |
| `/var/lib/openshell/systemd/` (root, 0711) | the state directory; one directory per sandbox ([chapter 6](06-operation.md#files)) |
| `/.openshell/supervisor`, `/.openshell/channel`, `/sandbox`, `/etc/openshell/auth/sandbox.jwt`, `/etc/openshell/tls/client/ca.crt` | mount points: empty on the host, and inside a sandbox's units the sandbox's files. They are the paths the stock programs use in a container |
| `/etc/userdb` | systemd's directory for user records under `/etc`; made where it is missing. A workload's unit mounts its sandbox's record over it, so records an administrator keeps there are not seen in a sandbox |
| `/run/openshell-systemd-helper/helper.sock` (0600, the gateway's user) | the helper's socket |
| `/run/user/<uid>/openshell/systemd-driver.sock` (0600) | the driver's socket |
| `/run/openshell-sandbox/<ID>/` (0700) | a supervisor's runtime directory |
| `/run/systemd/system/openshell-sandbox-{supervisor,workload}@<ID>.service.d/40-user.conf` | the helper's drop-ins with a sandbox's user |
| `…/50-selinux.conf`, in the same directories | the helper's drop-ins with a sandbox's SELinux level; only where the module is in use |
| `/run/userdb/osb-<UID>.user`, `osb-<UID>.group`, and the links `<UID>.user`, `<UID>.group` | the records that name a sandbox's user and group for systemd's user database |
| `/usr/share/selinux/packages/targeted/openshell_systemd.pp.bz2` | the policy module |
| `/usr/share/openshell-driver-systemd/docs/` | this guide (`openshell-driver-systemd-docs`) |
| `/usr/share/mcp-docs/collections.d/openshell-driver-systemd.yaml` | the guide's collection file for mcp-docs |
| `/usr/libexec/openshell-driver-systemd/tests/` | the functional tests (`openshell-driver-systemd-tests`) |

The package installs nothing in `/etc` as a file of its own and changes
nothing there on an update. The two directories and two empty files
below `/etc/openshell`, and `/etc/userdb`, are made by
`systemd-tmpfiles` where they are missing.

## The users {#user}

| User | What |
|---|---|
| `osb-<UID>` | one sandbox's user and group, from the helper's range: home `/sandbox`, shell `/usr/sbin/nologin`, in no other group. It exists as a record in `/run/userdb` and as the owner of the sandbox's files, not in `/etc/passwd` |
| `openshell-sandbox` | a system user and group of the package, home `/`, shell `/usr/sbin/nologin`. The unit templates name it and the helper's drop-in replaces it; it owns nothing. Do not give it files or groups |

## SELinux types {#selinux}

Module `openshell_systemd`.

| Type | For |
|---|---|
| `openshell_sandbox_t` | domain of a workload |
| `openshell_supervisor_t` | domain of a supervisor |
| `openshell_systemd_helper_t` | domain of the helper |
| `openshell_sandbox_netns_t` | domain of the namespace unit's command |
| `openshell_sandbox_exec_t`, `openshell_supervisor_exec_t`, `openshell_systemd_helper_exec_t` | the programs; the launcher has the supervisor's |
| `openshell_sandbox_unit_file_t` | the sandbox unit templates and the helper's drop-ins |
| `openshell_systemd_var_lib_t` | the state directory and `/.openshell` |
| `openshell_supervisor_conf_t` | a sandbox's `supervisor/` and `etc/`; a security file |
| `openshell_sandbox_channel_t` | a sandbox's `channel/` and `control/` |
| `openshell_sandbox_workspace_t` | a workspace, and `/sandbox` |
| `openshell_supervisor_runtime_t` | `/run/openshell-sandbox` |
| `openshell_sandbox_userdb_t` | the records that name the sandboxes' users, in `/run/userdb` and in a sandbox's `userdb/`; readable by every domain, as the users' file is |
| `openshell_sandbox_runtime_t` | the workload's own files under `/run` |
| `openshell_sandbox_tmp_t`, `openshell_supervisor_tmp_t` | their temporary files |
| `openshell_sandbox_devpts_t` | a workload's pseudo-terminals |
| `openshell_systemd_helper_runtime_t` | the helper's socket and its directory |

Both sandbox domains are constrained by categories (`mcs_constrained`).
A sandbox's level is `s0:cN,cM`, two different categories out of
`c0`–`c1023`.

## Packages and versions {#packages}

| Package | Requires |
|---|---|
| `openshell-driver-systemd` | `systemd >= 257`; the `-selinux` package where `selinux-policy-targeted` is installed |
| `openshell-driver-systemd-selinux` | the main package of the same version, `selinux-policy-targeted` |
| `openshell-driver-systemd-docs` | nothing; supplements the main package where `mcp-docs` is installed |
| `openshell-driver-systemd-tests` | the main package and the tools the tests use |

The OpenShell release a package is built against is in its description
(`rpm -qi openshell-driver-systemd`): **0.1.9** from version 0.3 of the
driver on, **0.1.8** for versions 0.1 and 0.2. The driver uses crates of OpenShell that OpenShell does not
promise to keep stable, so a package is built, and tested, against that
one release only. `openshell gateway info` shows the driver's own
version as the gateway sees it.
