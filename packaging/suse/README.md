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

## Releases

Each release has a branch `release-X.Y` and tags `vX.Y.Z`. Pushing a tag
publishes a GitHub release (`.github/workflows/release.yml`) with the
source files `make-sources` writes, the spec and `SHA256SUMS`, and the
tag's section of `CHANGELOG.md` as its notes. A build service project
takes those files as they are.

To release X.Y.0:

1. From the latest main, branch `release-X.Y`. CI runs on it as on main.
2. The release commit, a pull request against that branch, writes the
   section `## vX.Y.Z — <date>` into `CHANGELOG.md` from the fragments in
   `changes.d/` (`tools/changelog release vX.Y.Z`, which removes them),
   with `changes.d/intro.md` as its introduction: what the release is
   about and what to know when upgrading. The spec, `Cargo.toml` and
   `Cargo.lock` already say X.Y.Z.
3. `tools/check-release vX.Y.Z` checks that those three and the first
   `CHANGELOG.md` section name the release, that no fragment is left in
   `changes.d/`, and that the tree holds sources only. A maintainer then
   tags the branch and pushes the tag:
   `git tag -a vX.Y.Z -m "OpenShell systemd compute driver X.Y.Z"`,
   `git push origin vX.Y.Z`. The Release workflow runs the same check
   before it builds anything, and afterwards downloads what it published
   and compares it with `SHA256SUMS` and with `git archive` of the tag.
4. On main, the pull request that follows gives `CHANGELOG.md` the same
   section (`tools/changelog release` there, where the fragments still
   are) and sets the spec, `Cargo.toml` and `Cargo.lock` to the next
   version, `X.(Y+1).0`. New changes go into `changes.d/` again.

A fix release X.Y.Z+1 goes on the branch: the fix (from main, with its
fragment), then a release commit with its version and its `CHANGELOG.md`
section, and the same section on main afterwards.

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
