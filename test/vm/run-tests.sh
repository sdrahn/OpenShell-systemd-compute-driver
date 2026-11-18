#!/bin/bash
# SPDX-FileCopyrightText: Copyright (c) 2026 Daniel Rahn
# SPDX-License-Identifier: Apache-2.0
#
# run-tests.sh: the functional tests of the driver on an installed
# system: its packages, a stock OpenShell gateway (openshell-setup.sh),
# systemd and SELinux. Sandboxes are created, used, stopped, started and
# deleted through the gateway alone. Installed by the package
# openshell-driver-systemd-tests; test/openqa says how openQA runs it.
#
#   run-tests.sh --disposable [--results DIR] [--offline]
#                [--selinux-discovery]
#
# It installs packages, switches the gateway to the driver and restarts
# services, so it refuses to start without --disposable. Each check
# prints PASS or FAIL; the exit status is 1 if one failed. With
# --results, the log, the audit log's denials and the journals go to DIR,
# with every check as a result of its own: results.tap (TAP) and
# results.xml (JUnit, one test suite per section).
#
# The packages are expected to be installed, as a test system installs
# them from its repositories, and OpenShell's gateway to run as a service
# of a user (openshell-setup.sh next to this script does that).
#
# --offline leaves out the checks that reach a host on the Internet (the
# section "a connection the policy allows"): they are then SKIP.
#
# --selinux-discovery is for work on the policy module: the two domains
# are made permissive and every denial is logged, also those the policy
# would not log, so that one run shows what a workload needs. The denials
# are then listed, each kind once, and do not fail the run.
set -uo pipefail
user=${OPENSHELL_GATEWAY_USER:-osh}
disposable=
results=
discovery=
offline=
usage="usage: $0 --disposable [--results DIR] [--offline] [--selinux-discovery]"
while [ $# -gt 0 ]; do
	case $1 in
	--disposable) disposable=1 ;;
	--results) results=${2:?$usage}; shift ;;
	--offline) offline=1 ;;
	--selinux-discovery) discovery=1 ;;
	*) echo "$usage" >&2; exit 2 ;;
	esac
	shift
done
[ -n "$disposable" ] || { echo "$0 changes the system it runs on: pass --disposable on a machine that may be thrown away" >&2; exit 2; }
[ "$(id -u)" = 0 ] || { echo "$0 must run as root" >&2; exit 2; }
results=${results:-$(mktemp -d)}
mkdir -p "$results"
exec > >(tee "$results/test.log") 2>&1

uid=$(id -u "$user")
state=/var/lib/openshell/systemd
sock=/run/user/$uid/openshell/systemd-driver.sock
passed=0
failed=0
skipped=0
current="before the first section"
cases=$results/cases
rm -rf "$cases"
mkdir -p "$cases"

section() {
	current=$*
	printf '\n== %s\n' "$*"
}
# record STATUS DESCRIPTION [OUTPUT]: one result, for the TAP and JUnit
# files written at the end.
record() {
	printf '%s\n%s\n%s\n%s\n' "$current" "$1" "$2" "${3:-}" >"$cases/$(printf '%05d' $((passed + failed + skipped)))"
}
# check DESCRIPTION COMMAND…: PASS if the command succeeds.
check() {
	local what=$1 output
	shift
	if output=$("$@" 2>&1); then
		echo "PASS $what"
		passed=$((passed + 1))
		record pass "$what"
	else
		echo "FAIL $what"
		echo "$output" | head -n 20 | cut -c1-300 | sed 's/^/     | /'
		failed=$((failed + 1))
		record fail "$what" "$(echo "$output" | head -n 40 | cut -c1-500)"
	fi
}
# skip DESCRIPTION REASON: a check that this run leaves out.
skip() {
	echo "SKIP $1 ($2)"
	skipped=$((skipped + 1))
	record skip "$1" "$2"
}
# refuse DESCRIPTION COMMAND…: PASS if the command fails.
refuse() {
	local what=$1
	shift
	check "$what" not "$@"
}
not() { ! "$@"; }
asuser() { runuser -l "$user" -c "export XDG_RUNTIME_DIR=/run/user/$uid; $*"; }
phase() { openshell sandbox get "$1" 2>/dev/null | awk '$1 == "Phase:" {print $2}'; }
sandbox_id() { openshell sandbox get "$1" 2>/dev/null | awk '$1 == "Id:" {print $2}'; }
# wait_phase NAME PHASE [SECONDS]
wait_phase() {
	local i
	for i in $(seq $((${3:-180} / 5))); do
		[ "$(phase "$1")" = "$2" ] && return 0
		sleep 5
	done
	echo "phase of $1: $(phase "$1"), not $2"
	openshell sandbox get "$1" 2>&1 | tail -n 12
	return 1
}
label_of() { tr -d '\0' <"/proc/$1/attr/current"; }
# The categories of a process: what follows ":s0:" in its label.
categories_of() { label_of "$1" | sed -n 's/^.*:s0:\(c[0-9]*,c[0-9]*\)$/\1/p'; }
# reads_as CATEGORIES FILE: reads a file in the workload's domain, as the
# file's own user and with these categories, but outside every sandbox's
# namespaces. The file modes allow it: what is refused, the categories
# refuse.
reads_as() {
	systemd-run --quiet --wait --collect -p "User=$(stat -c %u "$2")" -p "Group=$(stat -c %g "$2")" \
		-p "SELinuxContext=system_u:system_r:openshell_sandbox_t:s0:$1" \
		/usr/local/bin/driver-test-cat "$2"
}
# user_of ID: the user the helper gave a sandbox.
user_of() { cat "$state/$1/uid" 2>/dev/null; }
# as_user UID COMMAND…: as that user and its group and nothing more, from
# here: outside the sandboxes' namespaces, SELinux domains and fences.
# What is refused, the kernel's own check of users refuses.
as_user() { setpriv --reuid "$1" --regid "$1" --clear-groups "${@:2}"; }
main_pid() { systemctl show "$1" -p MainPID --value; }
# eventually_ok SECONDS COMMAND…: the command succeeds within that time.
eventually_ok() {
	local deadline=$((SECONDS + $1))
	shift
	until "$@"; do
		[ "$SECONDS" -ge "$deadline" ] && return 1
		sleep 1
	done
}
# The user a sandbox's workload runs as right now.
user_of_running() { stat -c %u "/proc/$(main_pid "openshell-sandbox-workload@$1.service")"; }
# has FILE PATTERN / lacks FILE PATTERN: a line of the workload's report.
has() { grep -E -- "$2" "$1"; }
lacks() { ! grep -E -- "$2" "$1"; }
# From inside a sandbox.
inside() { timeout 90 openshell sandbox exec "$1" -- "${@:2}"; }

section "the packages"
check "the driver's package is installed" rpm -q openshell-driver-systemd
check "the policy module's package is installed" rpm -q openshell-driver-systemd-selinux
# It asks for selinux-policy's version, not the build's release: a host
# with an older release of the same version can install it.
check "it requires selinux-policy by version only, not the build's release" bash -c \
	"r=\$(rpm -q --requires openshell-driver-systemd-selinux | grep -E '^selinux-policy[a-z-]* >= '); [ -n \"\$r\" ] && ! grep -Eq ' >= [^ ]*-' <<<\"\$r\""
# The mount points are the package's, so that a read-only root (a
# transactional system) has them before the helper starts.
check "the package ships the sandboxes' mount points" rpm -qf /.openshell/supervisor /.openshell/channel /sandbox
for f in /usr/bin/openshell-driver-systemd /usr/libexec/openshell/openshell-systemd-helper \
	/usr/libexec/openshell/openshell-supervisor-fence \
	/usr/libexec/openshell/openshell-supervisor /usr/libexec/openshell/openshell-sandbox; do
	check "$f is installed" test -x "$f"
done
check "the driver names its version" sh -c '/usr/bin/openshell-driver-systemd --version | grep -E "[0-9]+\.[0-9]+\.[0-9]+"'
check "the user the unit templates name exists and is not root" sh -c 'u=$(id -u openshell-sandbox) && [ "$u" != 0 ]'
check "the unit templates are valid" systemd-analyze verify \
	/usr/lib/systemd/system/openshell-systemd-helper.service \
	/usr/lib/systemd/system/openshell-sandbox-netns@.service \
	/usr/lib/systemd/system/openshell-sandbox-supervisor@.service \
	/usr/lib/systemd/system/openshell-sandbox-workload@.service
check "SELinux is enforcing" sh -c '[ "$(getenforce)" = Enforcing ]'
check "the policy module is loaded" sh -c 'semodule -l | grep -qx openshell_systemd'
check "openshell-supervisor has its label" sh -c 'ls -Z /usr/libexec/openshell/openshell-supervisor | grep -q openshell_supervisor_exec_t'
check "openshell-sandbox has its label" sh -c 'ls -Z /usr/libexec/openshell/openshell-sandbox | grep -q openshell_sandbox_exec_t'
# The guide, where its package is installed.
guide=/usr/share/openshell-driver-systemd/docs
if rpm -q openshell-driver-systemd-docs >/dev/null 2>&1; then
	check "the guide's index is installed" test -s "$guide/user-guide/README.md"
	check "every chapter the index names is installed" sh -c "cd $guide/user-guide && for f in \$(grep -o '([0-9][0-9]-[a-z-]*\.md)' README.md | tr -d '()' | sort -u); do test -s \$f || { echo \$f; exit 1; }; done"
	check "the security review the guide links to is installed" sh -c "ls $guide/security-review-*.md"
	check "the guide's collection file names the directory of the guide" grep -qx "root: $guide" /usr/share/mcp-docs/collections.d/openshell-driver-systemd.yaml
	check "the guide names the OpenShell release the tests are written for" sh -c "v=\$(head -n1 $(dirname "$0")/openshell-version) && grep -qF \"**\${v#v}**\" $guide/user-guide/08-reference.md"
else
	skip "the guide's index is installed" "openshell-driver-systemd-docs is not installed"
	skip "every chapter the index names is installed" "openshell-driver-systemd-docs is not installed"
	skip "the security review the guide links to is installed" "openshell-driver-systemd-docs is not installed"
	skip "the guide's collection file names the directory of the guide" "openshell-driver-systemd-docs is not installed"
	skip "the guide names the OpenShell release the tests are written for" "openshell-driver-systemd-docs is not installed"
fi
# What an agent works with (the section "what an agent does").
check "the tools of an agent's work install" zypper -n in --no-recommends git-core gcc make python3 tar gzip util-linux iproute2 procps
if [ -n "$discovery" ]; then
	echo "SELinux discovery: the two domains are permissive, every denial is logged"
	zypper -n in --no-recommends policycoreutils-python-utils >/dev/null 2>&1
	semanage permissive -a openshell_sandbox_t
	semanage permissive -a openshell_supervisor_t
	semanage permissive -a openshell_systemd_helper_t
	semanage permissive -a openshell_sandbox_netns_t
	semodule -DB
fi

section "the helper"
mkdir -p /etc/systemd/system/openshell-systemd-helper.service.d
printf '[Service]\nEnvironment=OPENSHELL_SYSTEMD_HELPER_ALLOW_USER=%s\n' "$user" >/etc/systemd/system/openshell-systemd-helper.service.d/user.conf
systemctl daemon-reload
check "the helper starts" systemctl enable --now openshell-systemd-helper.service
sleep 2
check "the helper is active" systemctl is-active openshell-systemd-helper.service
check "the helper has its Landlock fence" sh -c 'journalctl -u openshell-systemd-helper.service --no-pager -o cat | grep -q "landlock.*Enforced"'
# The same program, started from here: not in the helper's SELinux domain,
# so what it is refused, the fence alone refuses.
echo "--- the helper's test of its fence"
/usr/libexec/openshell/openshell-systemd-helper --allow-user "$user" --self-test 2>&1 | cut -c1-200
check "the fence refuses root what the helper does not need" /usr/libexec/openshell/openshell-systemd-helper --allow-user "$user" --self-test
refuse "without the fence, the same test fails" /usr/libexec/openshell/openshell-systemd-helper --allow-user "$user" --self-test --landlock off
check "the helper keeps few of root's rights" sh -c "c=\$(awk '\$1 == \"CapBnd:\" {print \$2}' /proc/\$(systemctl show openshell-systemd-helper.service -p MainPID --value)/status) && [ \"\$c\" = 000000000000001f ]"
check "the helper runs in its domain" sh -c "tr -d '\\0' </proc/\$(systemctl show openshell-systemd-helper.service -p MainPID --value)/attr/current | grep -q ':openshell_systemd_helper_t:'"
check "the sandbox units have their label" sh -c 'ls -Z /usr/lib/systemd/system/openshell-sandbox-workload@.service | grep -q openshell_sandbox_unit_file_t'
check "the mount points exist without the helper having made them" test -d /.openshell/channel -a -d /sandbox -a -f /etc/openshell/auth/sandbox.jwt
ask='
import socket, sys
s = socket.socket(socket.AF_UNIX)
s.connect("/run/openshell-systemd-helper/helper.sock")
s.sendall(sys.argv[1].encode())
s.shutdown(socket.SHUT_WR)
print(s.recv(65536).decode())'
check "the helper answers the gateway's user" sh -c "runuser -u $user -- python3 -c '$ask' '\"describe\"' | grep -q described"
refuse "the helper does not answer root" sh -c "python3 -c '$ask' '\"describe\"' | grep -q described"
refuse "the helper's socket is closed to other users" runuser -u nobody -- python3 -c "$ask" '"describe"'
check "the helper refuses a path for a sandbox ID" sh -c "runuser -u $user -- python3 -c '$ask' '{\"delete\":{\"sandbox_id\":\"../../etc\"}}' | grep -q error"

section "the driver and the gateway"
# The certificate authority supervisors verify the gateway with.
ca=$(find "/home/$user/.local" "/home/$user/.config" -name ca.crt 2>/dev/null | head -n 1)
echo "the gateway's CA: ${ca:-not found}"
check "the gateway's CA certificate is found" test -n "$ca"
install -m 0644 "$ca" /etc/openshell-gateway-ca.crt
asuser "mkdir -p ~/.config/systemd/user/openshell-driver-systemd.service.d && printf '[Service]\nEnvironment=OPENSHELL_SYSTEMD_TLS_CA=/etc/openshell-gateway-ca.crt\nEnvironment=OPENSHELL_GATEWAY_NAME=openshell\n' >~/.config/systemd/user/openshell-driver-systemd.service.d/gateway.conf && systemctl --user daemon-reload"
check "the driver starts as the gateway's user" asuser "systemctl --user enable --now openshell-driver-systemd.service"
sleep 3
check "the driver is active" asuser "systemctl --user is-active openshell-driver-systemd.service"
check "the driver's socket is the gateway user's alone" sh -c "[ \"\$(stat -c '%U %a' $sock)\" = '$user 600' ]"
cfg=/home/$user/.config/openshell/gateway.toml
sed -i 's/^compute_driver = .*/compute_driver = "systemd"/' "$cfg"
printf '\n[openshell.drivers.systemd]\nsocket_path = "%s"\n' "$sock" >>"$cfg"
asuser "systemctl --user restart openshell-gateway.service"
for _ in $(seq 30); do
	openshell status >/dev/null 2>&1 && break
	sleep 2
done
check "the gateway runs with the driver" openshell status
check "the gateway names the driver" sh -c 'openshell gateway info | grep -q "openshell-driver-systemd"'

section "a sandbox"
# What the workload reports about itself, into its workspace.
report='exec >/sandbox/report.txt 2>&1
echo "context: $(id -Z)"
echo "user: $(id -un)"
grep -E "^(CapEff|CapBnd|NoNewPrivs|Seccomp):" /proc/self/status | tr "\t" " "
echo "interfaces: $(tail -n +3 /proc/net/dev | cut -d: -f1 | tr -d " " | tr "\n" " ")"
echo "write /etc: $(touch /etc/driver-test 2>&1 && echo WRITTEN)"
echo "write /usr: $(touch /usr/driver-test 2>&1 && echo WRITTEN)"
echo "write /tmp: $(touch /tmp/driver-test 2>&1 && echo WRITTEN)"
echo "write /sandbox: $(touch /sandbox/driver-test 2>&1 && echo WRITTEN)"
echo "read /etc/shadow: $(head -c 20 /etc/shadow 2>&1 | head -n 1)"
echo "state directory: $(ls /var/lib/openshell/systemd 2>&1 | head -n 1)"
echo "supervisor files: $(ls /.openshell/supervisor 2>&1 | head -n 1)"
echo "token: $(head -c 10 /etc/openshell/auth/sandbox.jwt 2>&1 | head -n 1)"
echo "direct tcp: $( (exec 3<>/dev/tcp/1.1.1.1/80) 2>&1 && echo OPEN)"
echo "mount: $(mount -t tmpfs none /tmp 2>&1 | head -n 1)"
echo "unshare: $(unshare -n true 2>&1 | head -n 1)"
echo "environment: ${DRIVER_TEST_ENV:-none} ${DRIVER_TEST_FROM:-none}"
unix() { python3 -c "
import socket, sys
s = socket.socket(socket.AF_UNIX, getattr(socket, sys.argv[2]))
s.connect(sys.argv[1])
print(\"CONNECTED\")" "$1" "$2" 2>&1 | tail -n 1; }
echo "dbus: $(unix /run/dbus/system_bus_socket SOCK_STREAM)"
echo "systemd: $(unix /run/systemd/private SOCK_STREAM)"
echo "journald: $(unix /run/systemd/journal/socket SOCK_DGRAM)"
echo "report done"
exec sleep 3600'
name=functional
# The sandbox's own variables, given with --env and, from this shell's
# environment, with --env-from.
export DRIVER_TEST_FROM=from-env-from
check "a sandbox is created" timeout 300 openshell sandbox create --name "$name" \
	--env DRIVER_TEST_ENV=from-env --env-from DRIVER_TEST_FROM --detach -- bash -c "$report"
check "the sandbox becomes ready" wait_phase "$name" Ready
id=$(sandbox_id "$name")
echo "sandbox id: $id"
d=$state/$id
r=$d/workspace/report.txt
for _ in $(seq 30); do grep -q '^report done' "$r" 2>/dev/null && break; sleep 2; done
echo "--- the workload's report"
cat "$r" 2>&1 | cut -c1-200

section "the units"
sup=$(main_pid "openshell-sandbox-supervisor@$id.service")
wl=$(main_pid "openshell-sandbox-workload@$id.service")
uid=$(user_of "$id")
echo "the sandbox's user: ${uid:-none}; passwd in nsswitch.conf: $(grep -h '^passwd' /etc/nsswitch.conf /usr/etc/nsswitch.conf 2>/dev/null | head -n 1); /run/userdb: $(ls -Zd /run/userdb 2>&1); userdbd: $(systemctl is-active systemd-userdbd.socket 2>&1)"
ls -lZ /run/userdb 2>&1 | cut -c1-200
check "the sandbox has a user of its own, from the helper's range" sh -c "[ '$uid' -ge 70000 ] && [ '$uid' -le 79999 ]"
check "no account in the users' file has that number" sh -c "! grep -q '^[^:]*:[^:]*:$uid:' /etc/passwd"
check "the host knows the user by its name" sh -c "[ \"\$(id -un $uid)\" = osb-$uid ]"
check "and its group" sh -c "[ \"\$(getent group $uid | cut -d: -f1)\" = osb-$uid ]"
check "the drop-ins name the user for both units" sh -c "grep -qx User=$uid /run/systemd/system/openshell-sandbox-supervisor@$id.service.d/40-user.conf /run/systemd/system/openshell-sandbox-workload@$id.service.d/40-user.conf"
check "the namespace's unit names its domain" sh -c "systemctl show openshell-sandbox-netns@$id.service -p SELinuxContext --value | grep -q ':openshell_sandbox_netns_t:'"
check "the namespace's unit has done its work" sh -c "[ \"\$(systemctl show openshell-sandbox-netns@$id.service -p Result --value)\" = success ]"
# The unit's process is gone by now. What its domain allows is shown by a
# program of the system started the same way: root, and that domain.
in_netns_domain() {
	systemd-run --quiet --wait --collect -p SELinuxContext=system_u:system_r:openshell_sandbox_netns_t:s0 "$@"
}
check "in the namespace unit's domain, a network setting is read" in_netns_domain /usr/bin/test -r /proc/sys/net/ipv4/ip_forward
refuse "in the namespace unit's domain, root cannot read the users' file" in_netns_domain /usr/bin/test -r /etc/passwd
refuse "in the namespace unit's domain, root cannot write in /etc" in_netns_domain /usr/bin/touch /etc/driver-test-netns
check "the three units are active" systemctl is-active "openshell-sandbox-netns@$id.service" "openshell-sandbox-supervisor@$id.service" "openshell-sandbox-workload@$id.service"
check "the supervisor is the stock program, started by its launcher" sh -c "[ \"\$(readlink /proc/$sup/exe)\" = /usr/libexec/openshell/openshell-supervisor ] && systemctl show openshell-sandbox-supervisor@$id.service -p ExecStart --value | grep -q openshell-supervisor-fence"
check "the supervisor cannot gain privileges" sh -c "grep -qx 'NoNewPrivs:[[:space:]]*1' /proc/$sup/status"
check "the supervisor's unit holds no sandbox's files but through its own mounts" sh -c "[ -z \"\$(nsenter -t $sup -m ls /var/lib/openshell)\" ]"
check "the supervisor's unit holds its own runtime directory alone" sh -c "[ \"\$(nsenter -t $sup -m ls /run/openshell-sandbox)\" = $id ]"
echo "the workload's /run: $(nsenter -t "$wl" -m ls /run 2>&1 | tr '\n' ' ')"
refuse "the workload's unit holds the host's D-Bus socket" nsenter -t "$wl" -m test -e /run/dbus/system_bus_socket
refuse "the workload's unit holds the journal's socket" nsenter -t "$wl" -m test -e /run/systemd/journal/socket
refuse "the workload's unit holds the supervisors' runtime directories" nsenter -t "$wl" -m test -e /run/openshell-sandbox
check "the workload has shared memory of its own" sh -c "[ \"\$(readlink /proc/$wl/ns/ipc)\" != \"\$(readlink /proc/1/ns/ipc)\" ]"
check "the supervisor runs in its domain" sh -c "tr -d '\\0' </proc/$sup/attr/current | grep -q ':openshell_supervisor_t:'"
check "the workload runs in its domain" sh -c "tr -d '\\0' </proc/$wl/attr/current | grep -q ':openshell_sandbox_t:'"
check "the supervisor runs as the sandbox's user" sh -c "[ \"\$(stat -c %u:%g /proc/$sup)\" = $uid:$uid ]"
check "the workload runs as the sandbox's user" sh -c "[ \"\$(stat -c %u:%g /proc/$wl)\" = $uid:$uid ]"
check "the workspace is the sandbox's user's alone" sh -c "[ \"\$(stat -c '%u %a' $d/workspace)\" = '$uid 700' ]"
check "the workload has a network namespace of its own" sh -c "[ \"\$(readlink /proc/$wl/ns/net)\" != \"\$(readlink /proc/1/ns/net)\" ]"
check "the workload has a process ID namespace of its own" sh -c "[ \"\$(readlink /proc/$wl/ns/pid)\" != \"\$(readlink /proc/1/ns/pid)\" ]"
check "the supervisor's files have their label" sh -c "ls -Zd $d/supervisor | grep -q openshell_supervisor_conf_t"
check "the workspace has its label" sh -c "ls -Zd $d/workspace | grep -q openshell_sandbox_workspace_t"
level=$(categories_of "$wl")
echo "categories: ${level:-none}"
check "the workload has a pair of categories" test -n "$level"
check "its supervisor has the same pair" sh -c "[ \"$(categories_of "$sup")\" = \"$level\" ]"
check "the workspace carries the pair" sh -c "ls -Zd $d/workspace | grep -q ':s0:$level '"
check "the supervisor's files carry the pair" sh -c "ls -Zd $d/supervisor | grep -q ':s0:$level '"
check "the credentials are the sandbox's user's alone" sh -c "[ \"\$(stat -c '%u %a' $d/etc/sandbox.jwt)\" = '$uid 600' ]"

section "what the workload can do"
check "the workload finished its report" has "$r" '^report done'
check "it runs in the workload's domain" has "$r" '^context: .*:openshell_sandbox_t:'
check "it runs as the sandbox's user, and knows its name" has "$r" "^user: osb-$uid\$"
check "it has no capabilities" has "$r" '^CapEff: *0000000000000000$'
check "it cannot gain privileges" has "$r" '^NoNewPrivs: *1$'
check "a seccomp filter is on" has "$r" '^Seccomp: *2$'
check "loopback is its only interface" has "$r" '^interfaces: lo $'
check "it has the sandbox's own variables (--env, --env-from)" has "$r" '^environment: from-env from-env-from$'
check "it writes its workspace" has "$r" '^write /sandbox: WRITTEN$'
check "it writes /tmp" has "$r" '^write /tmp: WRITTEN$'
check "it cannot write /etc" lacks "$r" '^write /etc: WRITTEN'
check "it cannot write /usr" lacks "$r" '^write /usr: WRITTEN'
check "it cannot read /etc/shadow" has "$r" '^read /etc/shadow: .*(denied|No such)'
check "it does not see the helper's state" has "$r" '^state directory: .*(denied|No such)'
check "it does not see its supervisor's files" has "$r" '^supervisor files: .*(denied|No such)'
check "it cannot read its supervisor's token" has "$r" '^token: .*(denied|No such)'
check "it has no direct connection to the network" lacks "$r" '^direct tcp: .*OPEN'
check "it cannot mount" lacks "$r" '^mount: *$'
check "it cannot make a namespace" has "$r" '^unshare: .*(not permitted|denied)'
check "it cannot connect to the system D-Bus" lacks "$r" '^dbus: CONNECTED'
check "it cannot connect to systemd" lacks "$r" '^systemd: CONNECTED'
check "it cannot connect to journald" lacks "$r" '^journald: CONNECTED'

section "through the gateway"
check "a command runs in the sandbox, as its user" sh -c "timeout 90 openshell sandbox exec $name -- id -u | grep -qx $uid"
check "the user has its name there" sh -c "timeout 90 openshell sandbox exec $name -- id -un | grep -qx osb-$uid"
check "the sandbox has its own user's record" inside "$name" test -r "/etc/userdb/osb-$uid.user"
refuse "and no other's" inside "$name" sh -c 'ls /etc/userdb | grep -v "^osb-'"$uid"'\.\|^'"$uid"'\." | grep -q .'
refuse "the host's records of every sandbox's user are not there" inside "$name" sh -c 'ls /run/userdb 2>/dev/null | grep -q .'
check "its name service is the sandbox's" sh -c "timeout 90 openshell sandbox exec $name -- getent hosts example.com"
check "a command run in it has the sandbox's own variables" sh -c "timeout 90 openshell sandbox exec $name -- printenv DRIVER_TEST_ENV | grep -qx from-env"
check "an image is refused" sh -c "timeout 120 openshell sandbox create --name withimage --from registry.example/image:1 --detach -- true 2>&1 | grep -q 'takes no image'"
openshell sandbox delete withimage >/dev/null 2>&1

section "what an agent does"
# Ordinary work of a coding or administration agent, each in a shell of
# the sandbox. It must work with the policy enforcing, and leave no
# denial behind (the last section).
work() { check "$1" inside "$name" bash -c "cd /sandbox && $2"; }
work "git: a repository, a commit, a log" 'git init -q repo && cd repo && echo hello >file && git add file && git -c user.name=agent -c user.email=agent@example.invalid commit -q -m first && git log --oneline | grep -q first'
work "git: a diff and a branch" 'cd repo && echo more >>file && git diff --stat | grep -q file && git checkout -q -b topic && git status --short | grep -q file'
work "a C program is compiled and run" 'printf "#include <stdio.h>\nint main(void){puts(\"built\");return 0;}\n" >hello.c && gcc -o hello hello.c && ./hello | grep -qx built'
work "make runs a rule" 'printf "all:\n\t@echo made\n" >Makefile && make | grep -qx made'
work "a Python script reads, computes and writes" 'python3 -c "import json, hashlib, pathlib; pathlib.Path(\"out.json\").write_text(json.dumps({\"sha\": hashlib.sha256(b\"x\").hexdigest()}))" && grep -q sha out.json'
work "Python starts processes in parallel" 'python3 -c "
import subprocess, sys
ps = [subprocess.Popen([sys.executable, \"-c\", \"print(%d * 2)\" % n], stdout=subprocess.PIPE) for n in (1, 2, 3)]
print(sum(int(p.communicate()[0]) for p in ps))" | grep -qx 12'
work "Python makes a virtual environment" 'python3 -m venv --without-pip venv && venv/bin/python -c "import sys; print(sys.prefix)" | grep -q /sandbox/venv'
work "a port below 1024 is taken on loopback" 'python3 -c "
import socket
s = socket.socket(); s.bind((\"127.0.0.1\", 80)); print(\"bound\")" | grep -qx bound'
work "a server on loopback is reached" 'python3 -m http.server 8123 --bind 127.0.0.1 >/dev/null 2>&1 & sleep 2; python3 -c "import urllib.request; print(urllib.request.urlopen(\"http://127.0.0.1:8123/file\" if False else \"http://127.0.0.1:8123/\").status)" | grep -qx 200; r=$?; kill %1; exit $r'
work "a Unix socket in the workspace" 'python3 -c "
import socket, os
p = \"/sandbox/test.sock\"
s = socket.socket(socket.AF_UNIX); s.bind(p); s.listen(1)
c = socket.socket(socket.AF_UNIX); c.connect(p); a, _ = s.accept()
c.sendall(b\"ok\"); print(a.recv(2).decode()); os.unlink(p)" | grep -qx ok'
work "an archive is packed and unpacked" 'tar -czf repo.tar.gz repo && mkdir unpacked && tar -xzf repo.tar.gz -C unpacked && test -f unpacked/repo/file'
work "a script written in /tmp runs" 'printf "#!/bin/sh\necho ran\n" >/tmp/agent-script && chmod +x /tmp/agent-script && /tmp/agent-script | grep -qx ran'
work "temporary files" 't=$(mktemp) && echo x >$t && rm $t && d=$(mktemp -d) && touch $d/f && rm -r $d'
work "its own processes: list, signal, wait" 'sleep 300 >/dev/null 2>&1 & p=$!; ps -o pid= -p $p | grep -q . && kill $p; wait $p; [ $? = 143 ]'
work "the host is read: packages, release, kernel" 'rpm -q systemd | grep -q systemd && grep -q openSUSE /etc/os-release && uname -r | grep -q .'
work "the host is read: processes, memory, disks" 'ps aux | grep -q . && free | grep -q Mem && df / | grep -q /'
work "the host is read: network state" 'ip -br addr | grep -q lo && ss -ltn >/dev/null'
work "files are found and searched" 'find /usr/share/doc -maxdepth 1 | head -n 3 | grep -q . && grep -r hello repo/file'
# Not here: a pseudo-terminal from /dev/ptmx and files in /dev/shm.
# OpenShell's default sandbox policy withholds both from the workload,
# whatever this driver or its SELinux module allow.
# What stays out of reach, as before the work.
refuse "it still cannot ask systemd for a unit's state" inside "$name" systemctl status openshell-systemd-helper.service
refuse "it still cannot read the journal" inside "$name" sh -c 'journalctl -n 1 --no-pager 2>/dev/null | grep -q .'
# journald keeps a file for the sandbox's user and would let it read it.
check "the host has a journal of the sandbox's user" sh -c "journalctl _UID=$uid -n 1 --no-pager -o cat | grep -q ."
refuse "the workload does not see the journals' directory" inside "$name" sh -c 'ls /var/log/journal 2>/dev/null | grep -q .'

section "restarts under a running sandbox"
asuser "systemctl --user restart openshell-driver-systemd.service"
sleep 4
check "the sandbox is ready after the driver restarted" wait_phase "$name" Ready 60
check "a command runs after the driver restarted" inside "$name" id
check "the units were not restarted with the driver" sh -c "[ \"\$(systemctl show openshell-sandbox-workload@$id.service -p MainPID --value)\" = $wl ]"
asuser "systemctl --user restart openshell-gateway.service"
for _ in $(seq 30); do openshell status >/dev/null 2>&1 && break; sleep 2; done
# The gateway stops its sandboxes when it shuts down and starts them again.
check "the sandbox is ready after the gateway restarted" wait_phase "$name" Ready
check "a command runs after the gateway restarted" inside "$name" id

section "a connection the policy allows"
# A sandbox whose policy lets one program reach one host. The way there is
# the supervisor's: its proxy, its name service, the certificate authority
# it gives the workload, and how it tells which program is asking.
policy=$results/net-policy.yaml
cat >"$policy" <<'POLICY'
version: 1
filesystem_policy:
  include_workdir: true
  read_only: [/bin, /usr, /lib, /proc, /dev/urandom, /app, /etc, /var/log]
  read_write: [/sandbox, /tmp, /dev/null]
landlock:
  compatibility: best_effort
network_policies:
  example:
    name: example-read-only
    endpoints:
      - host: example.com
        port: 443
        protocol: rest
        enforcement: enforce
        access: read-only
    binaries:
      - { path: /usr/bin/curl }
      - { path: /sandbox/named-curl }
      - { path: /tmp/named-curl }
POLICY
if [ -z "$offline" ]; then
	check "curl is installed on the host" zypper -n in --no-recommends curl
	check "a sandbox with a network policy is created" timeout 300 openshell sandbox create --name net --policy "$policy" --detach -- sleep 3600
	check "the sandbox with a network policy becomes ready" wait_phase net Ready
	# fetch PROGRAM: the status of a request to the allowed host.
	fetch() { inside net "$1" -sS -o /dev/null -m 30 -w '%{http_code}' https://example.com/; }
	echo "curl: $(fetch /usr/bin/curl 2>&1 | tail -n 2 | cut -c1-200)"
	check "the program the policy names reaches the host" sh -c "timeout 90 openshell sandbox exec net -- /usr/bin/curl -sS -o /dev/null -m 30 -w '%{http_code}' https://example.com/ | grep -qx 200"
	refuse "it does not reach a host the policy does not name" inside net /usr/bin/curl -sS -o /dev/null -m 20 https://www.opensuse.org/
	check "copies of the program are made" inside net sh -c 'cp /usr/bin/curl /sandbox/named-curl && cp /usr/bin/curl /sandbox/other-curl && cp /usr/bin/curl /tmp/named-curl'
	echo "a copy the policy does not name: $(fetch /sandbox/other-curl 2>&1 | tail -n 2 | cut -c1-200)"
	refuse "a copy the policy does not name is refused" inside net /sandbox/other-curl -sS -o /dev/null -m 20 https://example.com/
	echo "a copy in the workspace that the policy names: $(fetch /sandbox/named-curl 2>&1 | tail -n 2 | cut -c1-200)"
	check "a copy in the workspace that the policy names reaches the host" sh -c "timeout 90 openshell sandbox exec net -- /sandbox/named-curl -sS -o /dev/null -m 30 -w '%{http_code}' https://example.com/ | grep -qx 200"
	echo "a copy in /tmp that the policy names: $(fetch /tmp/named-curl 2>&1 | tail -n 2 | cut -c1-200)"
	check "a copy in /tmp that the policy names reaches the host" sh -c "timeout 90 openshell sandbox exec net -- /tmp/named-curl -sS -o /dev/null -m 30 -w '%{http_code}' https://example.com/ | grep -qx 200"
	check "the sandbox with a network policy is deleted" timeout 120 openshell sandbox delete net
else
	skip "the program the policy names reaches the host" "--offline"
	skip "it does not reach a host the policy does not name" "--offline"
	skip "a copy the policy does not name is refused" "--offline"
	skip "a copy in the workspace that the policy names reaches the host" "--offline"
	skip "a copy in /tmp that the policy names reaches the host" "--offline"
fi

section "a host with a private CA"
# An HTTPS server on the host's loopback with a certificate of a CA of its
# own, which goes into the host's trust store. A sandbox reaches it as
# host.openshell.internal; its supervisor ends the connection's TLS and
# verifies the server with the host's CA certificates. No Internet.
pca=$results/private-ca
mkdir -p "$pca"
openssl req -x509 -newkey rsa:2048 -nodes -days 2 -subj "/CN=driver-test CA" \
	-keyout "$pca/ca.key" -out "$pca/ca.pem" >/dev/null 2>&1
openssl req -newkey rsa:2048 -nodes -subj "/CN=host.openshell.internal" \
	-keyout "$pca/server.key" -out "$pca/server.csr" >/dev/null 2>&1
printf 'subjectAltName=DNS:host.openshell.internal\n' >"$pca/ext"
check "a server certificate of a private CA is made" openssl x509 -req -in "$pca/server.csr" -CA "$pca/ca.pem" \
	-CAkey "$pca/ca.key" -CAcreateserial -days 2 -extfile "$pca/ext" -out "$pca/server.pem"
mkdir -p "$pca/www" && echo private >"$pca/www/index.html"
systemd-run --quiet --unit=driver-test-https -p WorkingDirectory="$pca/www" /usr/bin/python3 -c '
import http.server, ssl
server = http.server.HTTPServer(("127.0.0.1", 18444), http.server.SimpleHTTPRequestHandler)
context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
context.load_cert_chain("'"$pca"'/server.pem", "'"$pca"'/server.key")
server.socket = context.wrap_socket(server.socket, server_side=True)
server.serve_forever()'
check "the server answers on the host" eventually_ok 20 sh -c "curl -sS --cacert '$pca/ca.pem' --resolve host.openshell.internal:18444:127.0.0.1 https://host.openshell.internal:18444/ | grep -qx private"
install -m 0644 "$pca/ca.pem" /etc/pki/trust/anchors/driver-test-ca.pem
check "the private CA goes into the host's trust store" update-ca-certificates
cat >"$results/private-policy.yaml" <<'POLICY'
version: 1
filesystem_policy:
  include_workdir: true
  read_only: [/bin, /usr, /lib, /proc, /dev/urandom, /app, /etc, /var/log]
  read_write: [/sandbox, /tmp, /dev/null]
landlock:
  compatibility: best_effort
network_policies:
  private:
    name: private-ca
    endpoints:
      - host: host.openshell.internal
        port: 18444
        protocol: rest
        enforcement: enforce
        access: read-only
    binaries:
      - { path: /usr/bin/curl }
POLICY
check "a sandbox for the private host is created" timeout 300 openshell sandbox create --name private --policy "$results/private-policy.yaml" --detach -- sleep 3600
check "it becomes ready" wait_phase private Ready
echo "the private host from the sandbox: $(inside private /usr/bin/curl -sS -m 30 https://host.openshell.internal:18444/ 2>&1 | tail -n 2 | cut -c1-200)"
check "the sandbox reaches the host with the private CA" sh -c "timeout 90 openshell sandbox exec private -- /usr/bin/curl -sS -m 30 https://host.openshell.internal:18444/ | grep -qx private"
check "the sandbox for the private host is deleted" timeout 120 openshell sandbox delete private
systemctl stop driver-test-https.service >/dev/null 2>&1
rm -f /etc/pki/trust/anchors/driver-test-ca.pem
update-ca-certificates >/dev/null 2>&1

section "two sandboxes"
check "a second sandbox is created" timeout 300 openshell sandbox create --name second --detach -- sleep 600
check "the second sandbox becomes ready" wait_phase second Ready
idb=$(sandbox_id second)
supb=$(main_pid "openshell-sandbox-supervisor@$idb.service")
wlb=$(main_pid "openshell-sandbox-workload@$idb.service")
supa=$(main_pid "openshell-sandbox-supervisor@$id.service")
check "the second has a network namespace of its own" sh -c "[ \"\$(readlink /proc/$wlb/ns/net)\" != \"\$(readlink /proc/\$(systemctl show openshell-sandbox-workload@$id.service -p MainPID --value)/ns/net)\" ]"
uidb=$(user_of "$idb")
echo "users: $uid and ${uidb:-none}"
check "the second sandbox has a user of its own" sh -c "[ -n '$uidb' ] && [ '$uidb' != '$uid' ] && [ \"\$(stat -c %u /proc/$wlb)\" = '$uidb' ]"
# One sandbox's files against another's categories, where neither the
# namespaces nor the file modes stand in the way: the reader is given the
# file's own user.
levelb=$(categories_of "$wlb")
echo "categories: $level and ${levelb:-none}"
check "the second sandbox has a pair of categories" test -n "$levelb"
check "the two sandboxes have different categories" test "$level" != "$levelb"
check "the second sandbox writes a file" inside second sh -c 'echo theirs >/sandbox/theirs'
install -m 0755 /usr/bin/cat /usr/local/bin/driver-test-cat
chcon -t openshell_sandbox_exec_t /usr/local/bin/driver-test-cat
check "outside the namespaces, a sandbox's file is read with its categories" reads_as "$levelb" "$state/$idb/workspace/theirs"
refuse "outside the namespaces, it is refused with another sandbox's categories" reads_as "$level" "$state/$idb/workspace/theirs"
refuse "and so is the other supervisor's token" reads_as "$level" "$state/$idb/etc/sandbox.jwt"
rm -f /usr/local/bin/driver-test-cat
# One sandbox's user against another's files and processes, with nothing
# else in the way: from here there is no namespace, no SELinux domain and
# no fence. The kernel's check of users refuses.
check "a sandbox's user reads its own workspace" as_user "$uidb" /usr/bin/cat "$state/$idb/workspace/theirs"
refuse "one sandbox's user cannot read another's workspace" as_user "$uid" /usr/bin/cat "$state/$idb/workspace/theirs"
refuse "one sandbox's user cannot read another's token" as_user "$uid" /usr/bin/cat "$state/$idb/etc/sandbox.jwt"
refuse "one sandbox's user cannot list another supervisor's runtime directory" as_user "$uid" /usr/bin/ls "/run/openshell-sandbox/$idb"
check "a sandbox's user asks whether its own supervisor is there" as_user "$uidb" /usr/bin/kill -0 "$supb"
refuse "one sandbox's user cannot ask that of another's supervisor" as_user "$uid" /usr/bin/kill -0 "$supb"
refuse "one sandbox's user cannot signal another's supervisor" as_user "$uid" /usr/bin/kill -CONT "$supb"
refuse "one sandbox's user cannot signal another's workload" as_user "$uid" /usr/bin/kill -CONT "$wlb"
# The supervisor's fence, alone: its launcher as root, whom file modes
# refuse nothing, outside the supervisor's SELinux domain. What is
# refused, the fence refuses.
behind_fence_of() {
	/usr/libexec/openshell/openshell-supervisor-fence --landlock "$2" \
		--runtime-dir "/run/openshell-sandbox/$1" -- "${@:3}"
}
check "behind a supervisor's fence, its own runtime directory is listed" behind_fence_of "$idb" required /usr/bin/ls "/run/openshell-sandbox/$idb"
# The host's CA bundle, with which the supervisor verifies the hosts a
# sandbox reaches; on openSUSE a link into /var/lib/ca-certificates.
echo "the host's CA bundle: $(readlink -f /etc/ssl/ca-bundle.pem 2>&1)"
check "behind a supervisor's fence, the host's CA bundle is read" behind_fence_of "$idb" required /usr/bin/head -c 64 /etc/ssl/ca-bundle.pem
check "without the fence, root reads another sandbox's token" behind_fence_of "$id" off /usr/bin/cat "$state/$idb/etc/sandbox.jwt"
refuse "behind the fence, root cannot read another sandbox's token" behind_fence_of "$id" required /usr/bin/cat "$state/$idb/etc/sandbox.jwt"
refuse "behind the fence, root cannot read another sandbox's workspace" behind_fence_of "$id" required /usr/bin/cat "$state/$idb/workspace/theirs"
refuse "behind the fence, root cannot list the other supervisor's runtime directory" behind_fence_of "$id" required /usr/bin/ls "/run/openshell-sandbox/$idb"
# One supervisor's processes against another's by the categories alone: in
# the supervisor's SELinux domain, as the user of the process that is
# signalled, with the first sandbox's categories.
install -m 0755 /usr/bin/kill /usr/local/bin/driver-test-kill
chcon -t openshell_supervisor_exec_t /usr/local/bin/driver-test-kill
signals_as() {
	systemd-run --quiet --wait --collect -p "User=$(stat -c %u "/proc/$3")" -p "Group=$(stat -c %g "/proc/$3")" \
		-p "SELinuxContext=system_u:system_r:openshell_supervisor_t:s0:$1" \
		/usr/local/bin/driver-test-kill "$2" "$3"
}
check "in its domain, a supervisor's program signals a process of its own sandbox" signals_as "$levelb" -0 "$supb"
refuse "with another sandbox's categories, it cannot send that supervisor a signal" signals_as "$level" -CONT "$supb"
# Not a check: signal 0, which only asks whether a process is there, is
# not held to the categories by the distribution's policy. The sandboxes'
# users are what refuses it (above).
echo "another sandbox's categories, signal 0: $(signals_as "$level" -0 "$supb" && echo delivered || echo refused)"
rm -f /usr/local/bin/driver-test-kill
refuse "a workload cannot signal its own supervisor" inside "$name" sh -c "kill -0 $supa"
refuse "a workload cannot signal the other supervisor" inside "$name" sh -c "kill -0 $supb"
refuse "a workload cannot signal the other workload" inside "$name" sh -c "kill -0 $wlb"
refuse "a workload cannot read the other supervisor's environment" inside "$name" sh -c "cat /proc/$supb/environ"
refuse "a workload does not see the supervisors' runtime directory" inside "$name" sh -c 'ls /run/openshell-sandbox/* 2>/dev/null | grep -q .'
check "a workload sees few processes, all its own" inside "$name" sh -c '[ "$(ls -d /proc/[0-9]* | wc -l)" -lt 20 ]'
check "the second sandbox is still ready" sh -c "[ \"\$(openshell sandbox get second | awk '\$1 == \"Phase:\" {print \$2}')\" = Ready ]"
check "the second sandbox is deleted" timeout 120 openshell sandbox delete second
sleep 3
refuse "the second sandbox's files are gone" test -e "$state/$idb"
refuse "the second sandbox's user has no name any more" sh -c "ls /run/userdb | grep -q '^osb-$uidb\.\|^$uidb\.'"

section "a workload that dies"
p=$(main_pid "openshell-sandbox-workload@$id.service")
[ -n "$p" ] && [ "$p" -gt 1 ] && kill -9 "$p"
check "the gateway learns that the workload died" wait_phase "$name" Error 60

section "stop, start, delete"
check "the sandbox stops" timeout 120 openshell sandbox stop "$name"
check "the sandbox is stopped" wait_phase "$name" Stopped 60
refuse "no unit of a stopped sandbox is active" systemctl is-active --quiet "openshell-sandbox-supervisor@$id.service" "openshell-sandbox-workload@$id.service"
check "the sandbox starts again" timeout 120 openshell sandbox start "$name"
check "the sandbox is ready again" wait_phase "$name" Ready
check "its workspace was kept" inside "$name" test -e /sandbox/driver-test
check "it has the user it had" test "$(user_of_running "$id")" = "$uid"
check "its workspace is written again" inside "$name" touch /sandbox/after-restart
check "the sandbox is deleted" timeout 120 openshell sandbox delete "$name"
sleep 3
refuse "its files are gone" test -e "$d"
refuse "none of its units is left" sh -c "systemctl list-units --all --no-legend 'openshell-sandbox-*' | grep -q ."

section "SELinux denials"
cat /var/log/audit/audit.log* 2>/dev/null | grep 'avc:  denied' >"$results/avc.txt"
echo "audit records: $(cat /var/log/audit/audit.log* 2>/dev/null | wc -l); denials: $(wc -l <"$results/avc.txt")"
check "the audit log is being written" sh -c 'cat /var/log/audit/audit.log* | grep -q "type="'
# Those the tests provoke: the workload's report tries the system D-Bus
# socket and journald's, systemctl in a sandbox tries D-Bus, and the
# test's own cat tries another sandbox's files, and the programs started
# in the namespace unit's domain try /etc.
check "a refused read of another sandbox's file is in the audit log" grep -q 'comm="driver-test-cat".*openshell_sandbox_t' "$results/avc.txt"
grep -v -E 'comm="(python3|systemctl)" name="(system_bus_socket|socket)".*scontext=[^ ]*:openshell_sandbox_t:|comm="driver-test-cat" .*scontext=[^ ]*:openshell_sandbox_t:|comm="driver-test-kil" .*scontext=[^ ]*:openshell_supervisor_t:|comm="(test|touch)" .*scontext=[^ ]*:openshell_sandbox_netns_t:' "$results/avc.txt" >"$results/avc-unexpected.txt"
sed -E 's/^.*avc: +denied +//; s/ pid=[0-9]+//; s/ ino=[0-9]+//' "$results/avc-unexpected.txt" | sort | uniq -c | cut -c1-300
if [ -n "$discovery" ]; then
	echo "--- every kind of denial once: source target class { permissions } commands, paths"
	python3 -c '
import re, sys
seen = {}
for line in sys.stdin:
    m = re.search(r"\{ ([^}]*)\}", line)
    g = lambda k: (re.search(k + r"=(\S+)", line) or [None, "-"])[1]
    s, t = (g(k).split(":")[2] if g(k) != "-" else "-" for k in ("scontext", "tcontext"))
    e = seen.setdefault((s, t, g("tclass")), [set(), set(), set()])
    e[0].update(m.group(1).split() if m else [])
    e[1].add(g("comm").strip(chr(34)))
    e[2].add((g("path") if g("path") != "-" else g("name")).strip(chr(34)))
for (s, t, c), (perms, comms, paths) in sorted(seen.items()):
    print(s, t, c, "{", " ".join(sorted(perms)), "}", ",".join(sorted(comms))[:80], " ".join(sorted(paths))[:260])
' <"$results/avc.txt"
else
	check "nothing else was denied" test ! -s "$results/avc-unexpected.txt"
fi

journalctl -u openshell-systemd-helper.service --no-pager -o short-precise >"$results/helper.journal" 2>&1
journalctl --no-pager -o short-precise _UID="$uid" 2>&1 | grep -v 'podman\|otel.name' | tail -n 400 >"$results/user.journal"
journalctl --no-pager -o short-precise -b 2>&1 | grep -E 'openshell-sandbox-(netns|supervisor|workload)@|openshell-supervisor\[|openshell-sandbox\[' | tail -n 400 >"$results/units.journal"

# Every check as a result of its own, for a test system to read.
python3 - "$cases" "$results" <<'RESULTS'
import os, sys
from xml.sax.saxutils import escape, quoteattr

cases_dir, out = sys.argv[1], sys.argv[2]
cases = []
for name in sorted(os.listdir(cases_dir)):
    with open(os.path.join(cases_dir, name), errors="replace") as f:
        section, status, what, *detail = f.read().split("\n")
    cases.append((section, status, what, "\n".join(detail).strip()))

# XML 1.0 has no place for most control characters.
clean = lambda text: "".join(c for c in text if c in "\t\n" or ord(c) >= 32)

with open(os.path.join(out, "results.tap"), "w") as tap:
    tap.write("TAP version 13\n1..%d\n" % len(cases))
    for number, (section, status, what, detail) in enumerate(cases, 1):
        line = "%s %d - %s: %s" % ("not ok" if status == "fail" else "ok", number, section, what)
        if status == "skip":
            line += " # SKIP " + detail
        tap.write(line.replace("\n", " ") + "\n")
        if status == "fail":
            for text in detail.split("\n"):
                tap.write("# " + text + "\n")

sections = []
for case in cases:
    if not sections or sections[-1][0] != case[0]:
        sections.append((case[0], []))
    sections[-1][1].append(case)
with open(os.path.join(out, "results.xml"), "w") as xml:
    xml.write('<?xml version="1.0" encoding="UTF-8"?>\n<testsuites name="openshell-driver-systemd">\n')
    for section, members in sections:
        count = lambda status: sum(1 for member in members if member[1] == status)
        xml.write('  <testsuite name=%s tests="%d" failures="%d" skipped="%d">\n'
                  % (quoteattr(section), len(members), count("fail"), count("skip")))
        for _, status, what, detail in members:
            xml.write('    <testcase classname=%s name=%s' % (quoteattr(section), quoteattr(clean(what))))
            if status == "pass":
                xml.write('/>\n')
            elif status == "skip":
                xml.write('><skipped message=%s/></testcase>\n' % quoteattr(clean(detail)))
            else:
                xml.write('><failure message="failed">%s</failure></testcase>\n' % escape(clean(detail)))
        xml.write('  </testsuite>\n')
    xml.write('</testsuites>\n')
RESULTS
rm -rf "$cases"

printf '\n%d passed, %d failed, %d skipped\n' "$passed" "$failed" "$skipped"
[ "$failed" = 0 ]
