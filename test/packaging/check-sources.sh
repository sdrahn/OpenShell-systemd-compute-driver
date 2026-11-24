#!/bin/bash
# SPDX-FileCopyrightText: Copyright (c) 2026 Daniel Rahn
# SPDX-License-Identifier: Apache-2.0
#
# check-sources.sh DIR: the package's source files in DIR, as
# packaging/suse/make-sources or OBS's services from _service write them,
# are what the spec takes. The three files are there, the vendor tarball
# holds Cargo.lock, .cargo/config.toml and its vendor directory at the
# top, and, unpacked as the spec's %prep unpacks them, the driver's
# workspace resolves every crate offline. CI runs it on both ways of making them.
set -euo pipefail
[ $# = 1 ] || { echo "usage: $0 <directory>" >&2; exit 2; }
src=$(cd "$1" && pwd)
spec=$(cd "$(dirname "$0")/../../packaging/suse" && pwd)/openshell-driver-systemd.spec
release=$(awk '$1 == "%define" && $2 == "openshell_version" {print $3}' "$spec")
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
failed=0
fail() { echo "FAIL $*"; failed=1; }
ok() { echo "ok   $*"; }

shopt -s nullglob
drivers=("$src"/openshell-driver-systemd-*.tar.gz)
[ ${#drivers[@]} = 1 ] && ok "the driver's source: ${drivers[0]##*/}" ||
	fail "not one openshell-driver-systemd-*.tar.gz in $src: ${drivers[*]##*/}"
for f in "openshell-$release.tar.gz" vendor.tar.zst; do
	[ -s "$src/$f" ] && ok "$f" || fail "no $f in $src"
done
[ "$failed" = 0 ] || exit 1

# top TARBALL: the entries at the top of a zstd tarball.
top() { zstd -dc "$1" | tar -tf - | sed 's|^\./||; s|/.*||' | sort -u | tr '\n' ' '; }
for v in vendor; do
	got=$(top "$src/$v.tar.zst")
	want=".cargo Cargo.lock $v "
	[ "$got" = "$want" ] && ok "$v.tar.zst holds $want" || fail "$v.tar.zst holds \"$got\", not \"$want\""
done

# As %prep: the driver's tree with its vendor tarball, OpenShell's tree
# beside its crates, and the [patch] the spec adds.
cd "$work"
tar -xzf "${drivers[0]}"
cd openshell-driver-systemd-*/
tar -xzf "$src/openshell-$release.tar.gz"
zstd -dc "$src/vendor.tar.zst" | tar -xf -
cmp -s Cargo.lock <(tar -xzOf "${drivers[0]}" --wildcards '*/Cargo.lock') &&
	ok "vendor.tar.zst's Cargo.lock is the driver's" || fail "vendor.tar.zst's Cargo.lock differs from the driver's"
sed -n '/^%prep/,/^%build/p' "$spec" | sed -n '/^{$/,/^} >>\.cargo\/config\.toml$/p' |
	sed "s/%{openshell_version}/$release/g" >"$work/patch.sh"
bash "$work/patch.sh"
export CARGO_NET_OFFLINE=true
if CARGO_HOME=$PWD/.cargo cargo metadata --offline --format-version 1 >/dev/null 2>"$work/err"; then
	ok "the driver's crates resolve offline"
else
	fail "the driver's crates do not resolve offline:"; sed 's/^/     | /' "$work/err"
fi
exit "$failed"
