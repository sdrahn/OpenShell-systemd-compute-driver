# Packaging for SLES 16 and openSUSE Leap 16

`openshell-driver-systemd.spec` builds four packages:

- `openshell-driver-systemd`: the driver, its helper, the unit
  templates, the user `openshell-sandbox`, and `openshell-supervisor`
  and `openshell-sandbox` of the OpenShell release the driver is built
  against. OpenShell's own package does not install those two on a
  host: with its other drivers they come inside container images.
- `openshell-driver-systemd-selinux`: the policy module. It is
  installed with the first where SELinux's targeted policy is.
- `openshell-driver-systemd-docs`: the guide (`docs/user-guide`) and
  the security review in `/usr/share/openshell-driver-systemd/docs`,
  and `packaging/mcp-docs/openshell-driver-systemd.yaml` in
  `/usr/share/mcp-docs/collections.d/`. Not as `%doc`: a system
  installed without documentation would leave it out. It is installed
  with the first where `mcp-docs` is.
- `openshell-driver-systemd-tests`: the functional tests
  (`test/openqa/README.md`).

## Sources

The build needs no network. `make-sources <directory>` writes what it
needs:

| File | Content |
|---|---|
| `openshell-driver-systemd-<version>.tar.gz` | this repository |
| `openshell-<release>.tar.gz` | OpenShell's source at the release `tools/openshell-version` prints |
| `vendor.tar.zst` | the crates of both, and `.cargo/config.toml` |

Then, in that directory: `rpmbuild -ba openshell-driver-systemd.spec`
with `_sourcedir` set to it, or `osc build`. CI does the first in a
Leap 16 container, and the functional tests install what it built.

## A new OpenShell release

`tools/openshell-version vX.Y.Z`, then `cargo update -p openshell-core`
(and the other three crates), `make check`, and the functional tests.
The driver uses crates that OpenShell does not promise to keep stable,
so a new release can need changes here.

## Not decided yet

- The supervisor is built with OpenShell's default features, which
  include its telemetry and its bundled CA roots. A distribution may
  prefer `--no-default-features --features system-ca-roots`.
- Where the gateway's CA certificate is on a packaged gateway, so that
  the driver's user unit can name it without a drop-in.
