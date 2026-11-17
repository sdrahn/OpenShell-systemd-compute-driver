#!/bin/bash
# SPDX-FileCopyrightText: Copyright (c) 2026 Daniel Rahn
# SPDX-License-Identifier: Apache-2.0
#
# run-tests.sh: the functional tests of the driver on an installed
# system: the packages of rpms/ next to this script, a stock OpenShell
# gateway (openshell-setup.sh), systemd and SELinux. Sandboxes are
# created, used, stopped, started and deleted through the gateway alone.
#
#   run-tests.sh --disposable [--results DIR] [--selinux-discovery]
#
# It installs packages, switches the gateway to the driver and restarts
# services, so it refuses to start without --disposable. Each check
# prints PASS or FAIL; the exit status is 1 if one failed. With
# --results, the log, the audit log's denials and the journals go to DIR.
#
# --selinux-discovery is for work on the policy module: the two domains
# are made permissive and every denial is logged, also those the policy
# would not log, so that one run shows what a workload needs. The denials
# are then listed, each kind once, and do not fail the run.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
user=${OPENSHELL_GATEWAY_USER:-osh}
disposable=
results=
discovery=
while [ $# -gt 0 ]; do
	case $1 in
	--disposable) disposable=1 ;;
	--results) results=$2; shift ;;
	--selinux-discovery) discovery=1 ;;
	*) echo "usage: $0 --disposable [--results DIR] [--selinux-discovery]" >&2; exit 2 ;;
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

section() { printf '\n== %s\n' "$*"; }
# check DESCRIPTION COMMAND…: PASS if the command succeeds.
check() {
	local what=$1 output
	shift
	if output=$("$@" 2>&1); then
		echo "PASS $what"
		passed=$((passed + 1))
	else
		echo "FAIL $what"
		echo "$output" | head -n 20 | cut -c1-300 | sed 's/^/     | /'
		failed=$((failed + 1))
	fi
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
# reads_as CATEGORIES FILE: reads a file in the workload's domain, as its
# user and with these categories, but outside every sandbox's namespaces:
# what a workload that had left its namespaces could do.
reads_as() {
	systemd-run --quiet --wait --collect -p User=openshell-sandbox \
		-p "SELinuxContext=system_u:system_r:openshell_sandbox_t:s0:$1" \
		/usr/local/bin/driver-test-cat "$2"
}
main_pid() { systemctl show "$1" -p MainPID --value; }
# has FILE PATTERN / lacks FILE PATTERN: a line of the workload's report.
has() { grep -E -- "$2" "$1"; }
lacks() { ! grep -E -- "$2" "$1"; }
# From inside a sandbox.
inside() { timeout 90 openshell sandbox exec "$1" -- "${@:2}"; }

section "the packages"
check "the packages install" zypper -n --no-gpg-checks in --no-recommends "$here"/rpms/*.rpm
for f in /usr/bin/openshell-driver-systemd /usr/libexec/openshell/openshell-systemd-helper \
	/usr/libexec/openshell/openshell-supervisor /usr/libexec/openshell/openshell-sandbox; do
	check "$f is installed" test -x "$f"
done
check "the driver names its version" sh -c '/usr/bin/openshell-driver-systemd --version | grep -E "[0-9]+\.[0-9]+\.[0-9]+"'
check "the workloads' user exists and is not root" sh -c 'u=$(id -u openshell-sandbox) && [ "$u" != 0 ]'
check "the unit templates are valid" systemd-analyze verify \
	/usr/lib/systemd/system/openshell-systemd-helper.service \
	/usr/lib/systemd/system/openshell-sandbox-netns@.service \
	/usr/lib/systemd/system/openshell-sandbox-supervisor@.service \
	/usr/lib/systemd/system/openshell-sandbox-workload@.service
check "SELinux is enforcing" sh -c '[ "$(getenforce)" = Enforcing ]'
check "the policy module is loaded" sh -c 'semodule -l | grep -qx openshell_systemd'
check "openshell-supervisor has its label" sh -c 'ls -Z /usr/libexec/openshell/openshell-supervisor | grep -q openshell_supervisor_exec_t'
check "openshell-sandbox has its label" sh -c 'ls -Z /usr/libexec/openshell/openshell-sandbox | grep -q openshell_sandbox_exec_t'
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
check "a sandbox is created" timeout 300 openshell sandbox create --name "$name" --detach -- bash -c "$report"
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
check "the supervisor runs in its domain" sh -c "tr -d '\\0' </proc/$sup/attr/current | grep -q ':openshell_supervisor_t:'"
check "the workload runs in its domain" sh -c "tr -d '\\0' </proc/$wl/attr/current | grep -q ':openshell_sandbox_t:'"
check "the supervisor is not root" sh -c "[ \"\$(stat -c %U /proc/$sup)\" = openshell-sandbox ]"
check "the workload is not root" sh -c "[ \"\$(stat -c %U /proc/$wl)\" = openshell-sandbox ]"
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
check "the credentials are the workloads' user's alone" sh -c "[ \"\$(stat -c '%U %a' $d/etc/sandbox.jwt)\" = 'openshell-sandbox 600' ]"

section "what the workload can do"
check "the workload finished its report" has "$r" '^report done'
check "it runs in the workload's domain" has "$r" '^context: .*:openshell_sandbox_t:'
check "it runs as the workloads' user" has "$r" '^user: openshell-sandbox$'
check "it has no capabilities" has "$r" '^CapEff: *0000000000000000$'
check "it cannot gain privileges" has "$r" '^NoNewPrivs: *1$'
check "a seccomp filter is on" has "$r" '^Seccomp: *2$'
check "loopback is its only interface" has "$r" '^interfaces: lo $'
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
check "a command runs in the sandbox" sh -c "timeout 90 openshell sandbox exec $name -- id -un | grep -qx openshell-sandbox"
check "its name service is the sandbox's" sh -c "timeout 90 openshell sandbox exec $name -- getent hosts example.com"
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

section "two sandboxes"
check "a second sandbox is created" timeout 300 openshell sandbox create --name second --detach -- sleep 600
check "the second sandbox becomes ready" wait_phase second Ready
idb=$(sandbox_id second)
supb=$(main_pid "openshell-sandbox-supervisor@$idb.service")
wlb=$(main_pid "openshell-sandbox-workload@$idb.service")
supa=$(main_pid "openshell-sandbox-supervisor@$id.service")
check "the second has a network namespace of its own" sh -c "[ \"\$(readlink /proc/$wlb/ns/net)\" != \"\$(readlink /proc/\$(systemctl show openshell-sandbox-workload@$id.service -p MainPID --value)/ns/net)\" ]"
# One sandbox's files against another's categories, where neither the
# namespaces nor the file modes stand in the way: both run as one user.
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
check "the D-Bus attempt was denied by the policy" grep -q 'system_bus_socket.*openshell_sandbox_t' "$results/avc.txt"
grep -v -E 'comm="(python3|systemctl)" name="(system_bus_socket|socket)".*scontext=[^ ]*:openshell_sandbox_t:|comm="driver-test-cat" .*scontext=[^ ]*:openshell_sandbox_t:|comm="(test|touch)" .*scontext=[^ ]*:openshell_sandbox_netns_t:' "$results/avc.txt" >"$results/avc-unexpected.txt"
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

printf '\n%d passed, %d failed\n' "$passed" "$failed"
[ "$failed" = 0 ]
