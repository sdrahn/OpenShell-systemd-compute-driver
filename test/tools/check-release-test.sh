#!/bin/bash
# SPDX-FileCopyrightText: Copyright (c) 2026 Daniel Rahn
# SPDX-License-Identifier: Apache-2.0
#
# check-release-test.sh: tools/check-release on a repository made for the
# test: it accepts a commit whose files all name the release and names
# each file that does not. Part of `make check`.
set -euo pipefail

tools=$(cd "$(dirname "$0")/../../tools" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
failed=0

# A repository with the tools and the files of a release 1.2.0.
repo=$work/repo
mkdir -p "$repo/tools" "$repo/packaging/suse" "$repo/changes.d"
cp "$tools/check-release" "$tools/check-tree" "$repo/tools/"
cd "$repo"
git init -q
git config user.name test
git config user.email test@example.invalid
git config commit.gpgsign false
printf 'Name: openshell-driver-systemd\nVersion:        1.2.0\n' >packaging/suse/openshell-driver-systemd.spec
printf '[package]\nname = "openshell-driver-systemd"\nversion = "1.2.0"\n\n[dependencies]\nother = { version = "9.9.9" }\n' >Cargo.toml
printf '[[package]]\nname = "other"\nversion = "9.9.9"\n\n[[package]]\nname = "openshell-driver-systemd"\nversion = "1.2.0"\n' >Cargo.lock
printf '# Changelog\n\n## v1.2.0 — 2026-01-02\n\n- A change.\n\n## v1.1.0 — 2025-12-01\n' >CHANGELOG.md
echo "# changes.d" >changes.d/README.md
git add -A
git commit -q -m release

# expect STATUS PATTERN DESCRIPTION TAG: the exit status of check-release
# for HEAD, and a line of its output.
expect() {
	local status=$1 pattern=$2 what=$3 tag=$4 out got=0
	out=$(tools/check-release "$tag" HEAD 2>&1) || got=$?
	if [ "$got" = "$status" ] && echo "$out" | grep -q -- "$pattern"; then
		echo "ok   $what"
	else
		echo "FAIL $what: status $got, not $status, or no \"$pattern\" in:"
		echo "$out" | sed 's/^/     | /'
		failed=1
	fi
}
# change DESCRIPTION: commit what the test changed.
change() { git add -A && git commit -q -m "$1"; }

expect 0 'is ready for v1.2.0' "a commit whose files name the release" v1.2.0
expect 1 'not "## v1.2.0-rc1 ' "a release candidate, which needs a section of its own" v1.2.0-rc1
expect 2 'not a tag' "something that is no tag" 1.2.0
expect 1 'spec says Version: 1.2.0, not 1.3.0' "another version than the spec's" v1.3.0
expect 1 'first section of CHANGELOG.md' "another version than the changelog's" v1.3.0

sed -i 's/^version = "1.2.0"/version = "1.2.1"/' Cargo.toml
change "Cargo.toml ahead"
expect 1 'Cargo.toml says version = "1.2.1"' "Cargo.toml at another version" v1.2.0
git reset -q --hard HEAD~1

sed -i '$ s/1.2.0/1.1.0/' Cargo.lock
change "Cargo.lock behind"
expect 1 'Cargo.lock has openshell-driver-systemd at 1.1.0' "Cargo.lock at another version" v1.2.0
git reset -q --hard HEAD~1

echo "- Not released yet." >changes.d/topic.fixed.md
change "a fragment"
expect 1 'changes.d still has changes.d/topic.fixed.md' "a fragment that is not in the changelog" v1.2.0
git reset -q --hard HEAD~1

printf '\0\1\2' >program
change "a binary file"
expect 1 'program is a binary file' "a tree with a binary file" v1.2.0
git reset -q --hard HEAD~1

expect 0 'is ready for v1.2.0' "the first commit again" v1.2.0
exit "$failed"
