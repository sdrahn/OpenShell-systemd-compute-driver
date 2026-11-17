#!/bin/bash
# SPDX-FileCopyrightText: Copyright (c) 2026 Daniel Rahn
# SPDX-License-Identifier: Apache-2.0
#
# Sets up OpenShell in the test VM as its installer intends: rootless
# Podman, the gateway a systemd user service of an ordinary user (osh,
# lingering), and that gateway registered for root too, who runs the
# tests. Run as root.
#
#   openshell-setup.sh [--from-repositories]
set -euo pipefail
user=osh
log() { echo "[openshell-setup] $*"; }

log "Podman"
zypper -n in --no-recommends podman curl jq audit dnsmasq >/tmp/zypper-osh.log 2>&1 ||
	{ tail -n 30 /tmp/zypper-osh.log; exit 1; }

log "cgroup controllers for user sessions (rootless Podman sets memory and swap limits)"
# Rootless Podman needs the controllers delegated to the user's systemd;
# OpenShell's sandboxes get memory limits (memory.max, memory.swap.max).
mkdir -p /etc/systemd/system/user@.service.d
printf '[Service]\nDelegate=cpu cpuset io memory pids\n' >/etc/systemd/system/user@.service.d/delegate.conf
systemctl daemon-reload

log "user $user (rootless Podman, lingering)"
id "$user" >/dev/null 2>&1 || useradd -m "$user"
grep -q "^$user:" /etc/subuid || usermod --add-subuids 200000-265535 --add-subgids 200000-265535 "$user"
loginctl enable-linger "$user"
uid=$(id -u "$user")
for _ in $(seq 60); do [ -d "/run/user/$uid" ] && break; sleep 1; done
[ -d "/run/user/$uid" ] || { echo "no user manager for $user" >&2; exit 1; }
# A user manager started before the drop-in runs without the delegation.
systemctl restart "user@$uid.service"
sleep 2
d=/sys/fs/cgroup/user.slice/user-$uid.slice/user@$uid.service
log "controllers for $user: $(cat "$d/cgroup.controllers" 2>&1); swap limit: $([ -e "$d/memory.swap.max" ] && echo yes || echo no)"

# OpenShell itself. By default the release the tests are written for
# (openshell-version next to this script), with its own installer, which
# gets the packages from the Internet, starts the gateway as a service of
# the user and registers it for that user. With --from-repositories the
# packages come from the system's repositories, as a test system has
# them, and the same three steps are done here.
osh_version=$(head -n1 "$(dirname "$0")/openshell-version")
if [ "${1:-}" = --from-repositories ]; then
	log "OpenShell from the repositories"
	zypper -n in --no-recommends openshell openshell-gateway >/tmp/zypper-openshell.log 2>&1 ||
		{ tail -n 30 /tmp/zypper-openshell.log; exit 1; }
	asuser() { runuser -l "$user" -c "export XDG_RUNTIME_DIR=/run/user/$uid; $*"; }
	asuser "systemctl --user daemon-reload && systemctl --user enable openshell-gateway && systemctl --user restart openshell-gateway"
	for _ in $(seq 30); do
		asuser "systemctl --user is-active --quiet openshell-gateway" && break
		sleep 2
	done
	asuser "openshell gateway add https://127.0.0.1:17670 --local --name openshell"
	log "installed: $(openshell --version), the tests are written for $osh_version"
else
	log "OpenShell $osh_version (its installer, for $user)"
	curl -LsSf --retry 3 --retry-all-errors "https://raw.githubusercontent.com/NVIDIA/OpenShell/$osh_version/install.sh" -o /tmp/openshell-install.sh
	for attempt in 1 2 3; do
		OPENSHELL_VERSION=$osh_version SUDO_USER=$user sh /tmp/openshell-install.sh && break
		[ "$attempt" = 3 ] && { echo "installing OpenShell $osh_version failed" >&2; exit 1; }
		sleep 20
	done
	openshell --version | grep -qF "${osh_version#v}" || { echo "OpenShell is not $osh_version: $(openshell --version)" >&2; exit 1; }
fi
openshell --version

log "the gateway registration for root"
mkdir -p /root/.config
rm -rf /root/.config/openshell
cp -a "/home/$user/.config/openshell" /root/.config/
openshell gateway list
openshell status
log "SELinux: $(getenforce)"
