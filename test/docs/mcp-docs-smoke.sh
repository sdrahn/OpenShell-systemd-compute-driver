#!/bin/sh
# SPDX-FileCopyrightText: Copyright (c) 2026 Daniel Rahn
# SPDX-License-Identifier: Apache-2.0
#
# mcp-docs-smoke.sh: check an installed openshell-driver-systemd-docs
# with mcp-docs (github.com/sdrahn/mcp-docs), as root (CI's openSUSE
# container): its files, and mcp-docs serving the collection
# openshell-driver-systemd from its collection file as an agent asks.
#
#   test/docs/mcp-docs-smoke.sh [MCP-DOCS]
set -eu

mcpdocs=${1:-mcp-docs}
name=openshell-driver-systemd
yaml=/usr/share/mcp-docs/collections.d/$name.yaml
docs=/usr/share/$name/docs
out=$(mktemp)
err=$(mktemp)
trap 'rm -f "$out" "$err"' EXIT
fail() {
	echo "mcp-docs-smoke: $*" >&2
	exit 1
}
call() { # id, tool, arguments
	echo "{\"jsonrpc\":\"2.0\",\"id\":$1,\"method\":\"tools/call\",\"params\":{\"name\":\"$2\",\"arguments\":$3}}"
}

for f in "$yaml" "$docs/security-review-0.1.md" "$docs/user-guide/README.md" "$docs/user-guide/07-troubleshooting.md"; do
	rpm -qf "$f" | grep -q "^$name-docs-" || fail "$f is not owned by $name-docs"
done

{
	echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}'
	echo '{"jsonrpc":"2.0","method":"notifications/initialized"}'
	call 2 find_sections '{"query":"limit the memory and CPU of a sandbox"}'
	call 3 find_sections '{"query":"let one program reach one host with a network policy"}'
	call 4 search "{\"query\":\"takes no image\",\"collection\":\"$name\"}"
	call 5 read_section "{\"doc\":\"$name/user-guide/05-security.md#landlock\"}"
	echo "{\"jsonrpc\":\"2.0\",\"id\":6,\"method\":\"resources/read\",\"params\":{\"uri\":\"docs://$name/user-guide/README.md\"}}"
	sleep 2
} | timeout 30 "$mcpdocs" >"$out" 2>"$err" || fail "the session failed: $(cat "$err")"

grep -q "serving .*$name=$docs" "$err" || fail "not served: $(cat "$err")"
for a in 'OpenShell systemd compute driver documentation' "Index: $name/user-guide/README.md" \
	"$name/user-guide/06-operation.md#limits" \
	"$name/user-guide/04-network-policy.md#example" \
	"$name/user-guide/03-sandboxes.md" \
	'best-effort' 'Common questions'; do
	grep -q -- "$a" "$out" || fail "no $a in the answers: $(cat "$out")"
done
echo "mcp-docs-smoke: $name-docs is served ($(find "$docs" -name '*.md' | wc -l) documents)"
