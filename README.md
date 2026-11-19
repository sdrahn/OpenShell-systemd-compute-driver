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

## The guide

[docs/user-guide](docs/user-guide/README.md) is the documentation, for
administrators and for those who work in a sandbox:

| Chapter | For |
|---|---|
| [1. What the driver is and when to use it](docs/user-guide/01-overview.md) | the parts, who enforces what, what it does not support |
| [2. Installing and setting up](docs/user-guide/02-installation.md) | requirements, packages, the three setup steps, checking, removing |
| [3. Working in a sandbox](docs/user-guide/03-sandboxes.md) | what a workload sees and can do, the workspace, differences from a container |
| [4. Network access and policy](docs/user-guide/04-network-policy.md) | a worked example of a policy |
| [5. The security model](docs/user-guide/05-security.md) | the layers, SELinux, Landlock, the known limits |
| [6. Operating the driver](docs/user-guide/06-operation.md) | units, files, logs, limits, restarts, backups |
| [7. Troubleshooting](docs/user-guide/07-troubleshooting.md) | symptoms and error messages |
| [8. Reference](docs/user-guide/08-reference.md) | every setting, unit, file and SELinux type |
| [9. The helper protocol](docs/user-guide/09-helper-protocol.md) | what crosses the privilege boundary |
| [10. What the tests hold](docs/user-guide/10-tested.md) | which statements a test checks, and which none does |

The package `openshell-driver-systemd-docs` installs it in
`/usr/share/openshell-driver-systemd/docs`, with a collection file for
[mcp-docs](https://github.com/sdrahn/mcp-docs), which then serves it to
the agents on the host.

## In short

A sandbox is three instances of unit templates, named by the sandbox's
ID:

| Unit | Runs | As | Network |
|---|---|---|---|
| `openshell-sandbox-netns@ID.service` | holds the sandbox's network namespace | root | a namespace with loopback only |
| `openshell-sandbox-supervisor@ID.service` | `openshell-supervisor` | the sandbox's user | the host's |
| `openshell-sandbox-workload@ID.service` | `openshell-sandbox`, which starts the agent | the sandbox's user | the namespace of the unit above |

Every sandbox runs as a host user of its own, which the helper takes
from a range of user IDs. The workload's root file system is the
host's, read-only; a sandbox has no image. All its egress goes through
its supervisor.

The gateway's driver socket accepts only the gateway's own user, who
cannot start system units, so there are two programs:
`openshell-driver-systemd`, the `ComputeDriver` service without
privilege, and `openshell-systemd-helper`, which runs as root, writes a
launch's files and starts and stops the units. A request to the helper
carries file contents and a sandbox ID, never a path, a unit name or a
command ([chapter 9](docs/user-guide/09-helper-protocol.md)).

Workloads, supervisors and the helper are confined by systemd's
settings, an optional SELinux module with categories per sandbox, and
Landlock ([chapter 5](docs/user-guide/05-security.md)).
[docs/security-review-0.1.md](docs/security-review-0.1.md) is the
review before the first release: what was found, what was fixed and
what stays.

## Install

On SLES 16 and openSUSE Leap 16: the package `openshell-driver-systemd`
(`packaging/suse`), then three steps: name the gateway's user for the
helper, start the driver as that user, and point the gateway's
`gateway.toml` at the driver's socket
([chapter 2](docs/user-guide/02-installation.md)).

## Build and tests

`make build` builds the programs, `make selinux` the policy module.
`make check` runs the format check, clippy, the unit tests and the
checks of the tree, the changelog and the guide. OpenShell does not
publish its crates, so Cargo fetches them from the release tag named in
`Cargo.toml`; the driver is built and tested against that one release.

The functional tests (`test/vm/run-tests.sh`) run on an installed
system and are shipped as the package `openshell-driver-systemd-tests`;
[test/openqa](test/openqa/README.md) says how openQA runs them.
`test/vm/run-vm.sh <rpm dir>` runs them in a disposable Leap 16 VM, as
CI does.

Released: 0.2.0 (branch `release-0.2`; 0.1.4 on `release-0.1`;
[CHANGELOG.md](CHANGELOG.md)). Main is 0.3.0 in development.

Changes are made as [AGENTS.md](AGENTS.md) says, for people and coding
agents alike.
