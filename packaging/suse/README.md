# Packaging for SLES 16 and openSUSE Leap 16

`openshell-driver-systemd.spec` builds four packages:

- `openshell-driver-systemd`: the driver, its helper, the unit
  templates and the user `openshell-sandbox`. It requires the packages
  `openshell-supervisor` and `openshell-sandbox` of the OpenShell
  release the driver is built against (`/usr/bin/openshell-supervisor`
  and `/usr/bin/openshell-sandbox`), which the sandboxes run. The
  functional tests take them from the OBS project `home:sdrahn:testing`
  (`test/vm/run-vm.sh`).
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

The build needs no network. Its three source files come from OBS's
source services (`_service`) or from `make-sources`, the same files laid
out the same way:

| File | Content |
|---|---|
| `openshell-driver-systemd-<version>.tar.gz` | this repository |
| `openshell-<release>.tar.gz` | OpenShell's source at the release `tools/openshell-version` prints, at the commit `Cargo.lock` locks |
| `vendor.tar.zst` | the driver's crates: `Cargo.lock`, `.cargo/config.toml` and `vendor/` |

The crates are vendored as `Cargo.lock` locks them, never updated in
passing. A new crate version is a change here, in `Cargo.lock`.

### In a build service project

As with Custos and mcp-gateway, `osc service manualrun` makes the source
files, here from the branch `_service` names:

```bash
osc mkpac openshell-driver-systemd
cd openshell-driver-systemd
cp <repo>/packaging/suse/{openshell-driver-systemd.spec,_service} .
osc service manualrun
osc vc -m "Initial package"
osc add *
osc commit
```

It needs `obs-service-tar_scm`, `obs-service-recompress`,
`obs-service-set_version` and `obs-service-cargo`, and the network. The
services refresh the sources only: copy the spec and `_service` again
whenever they change here. Before running them again, delete the
previous run's tarballs (`osc rm` them): `cargo_vendor` takes the first
tarball that matches. Its audit of the lock files stops the run on a
known vulnerability of a crate; that is fixed here, by updating the
crate, not in the project.

### With make-sources

`make-sources <directory>` writes the same three files and the spec, from
the commit checked out. Then, in that directory: `rpmbuild -ba
openshell-driver-systemd.spec` with `_sourcedir` set to it, or `osc
build`. CI does the first in a Leap 16 container, and the functional
tests install what it built. CI also runs the services of `_service` on
the commit under test (`test/packaging/run-services.py`, as `osc service
manualrun` runs them) and checks both sets of files against the spec
(`test/packaging/check-sources.sh`).

## Releases

Each release has a branch `release-X.Y` and tags `vX.Y.Z`. Pushing a tag
publishes a GitHub release (`.github/workflows/release.yml`) with the
source files `make-sources` writes, the spec and `SHA256SUMS`, and the
tag's section of `CHANGELOG.md` as its notes. A build service project
takes those files as they are.

To release X.Y.0:

1. From the latest main, branch `release-X.Y`. Its first commit makes
   `_service` build from `release-X.Y` with the version of its latest
   tag (`versionformat` `@PARENT_TAG@`, with the `versionrewrite-pattern`
   `v(.*)`), so that the package's version is `X.Y.Z`; on main it builds
   snapshots, `X.Y.Z+git<date>.<hash>`. CI runs on the branch as on main.
2. The release commit, a pull request against that branch, writes the
   section `## vX.Y.Z — <date>` into `CHANGELOG.md` from the fragments in
   `changes.d/` (`tools/changelog release vX.Y.Z`, which removes them),
   with `changes.d/intro.md` as its introduction: what the release is
   about and what to know when upgrading. The spec, `Cargo.toml` and
   `Cargo.lock` already say X.Y.Z.
3. `tools/check-release vX.Y.Z` checks that those three, the first
   `CHANGELOG.md` section and `_service` name the release, that no
   fragment is left in `changes.d/`, and that the tree holds sources
   only. A maintainer then
   tags the branch and pushes the tag:
   `git tag -a vX.Y.Z -m "OpenShell systemd compute driver X.Y.Z"`,
   `git push origin vX.Y.Z`. The Release workflow runs the same check
   before it builds anything, and afterwards downloads what it published
   and compares it with `SHA256SUMS` and with `git archive` of the tag.
4. On main, the pull request that follows gives `CHANGELOG.md` the same
   section (`tools/changelog release` there, where the fragments still
   are) and sets the spec, `Cargo.toml`, `Cargo.lock` and `_service`'s
   `versionformat` to the next version, `X.(Y+1).0`. New changes go
   into `changes.d/` again.

A fix release X.Y.Z+1 goes on the branch: the fix (from main, with its
fragment), then a release commit with its version and its `CHANGELOG.md`
section, and the same section on main afterwards.

A build service project that follows a release takes the spec and
`_service` of the release's tag, runs `osc service manualrun`, then
`osc vc -m "Update to X.Y.Z"` with the release's section, and commits:

```bash
tag=vX.Y.Z
for f in openshell-driver-systemd.spec _service; do
  curl -sSfL -o "$f" "https://raw.githubusercontent.com/sdrahn/OpenShell-systemd-compute-driver/$tag/packaging/suse/$f"
done
osc service manualrun
osc vc -m "Update to ${tag#v}"
osc commit
```

Or it takes the files the release published, as they are.

## A new OpenShell release

The packages `openshell-supervisor` and `openshell-sandbox` of that
release have to be in the repositories first: the driver requires them
at the same version. Then `tools/openshell-version vX.Y.Z`, then `cargo update -p openshell-core`
(and the other three crates), `tools/openshell-version vX.Y.Z` again
(it gives `_service` the commit `Cargo.lock` now locks), `make check`,
and the functional tests.
The driver uses crates that OpenShell does not promise to keep stable,
so a new release can need changes here.

## Not decided yet

- Where the gateway's CA certificate is on a packaged gateway, so that
  the driver's user unit can name it without a drop-in.
