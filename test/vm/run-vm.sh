#!/bin/bash
# SPDX-FileCopyrightText: Copyright (c) 2026 Daniel Rahn
# SPDX-License-Identifier: Apache-2.0
#
# run-vm.sh: the functional tests in a disposable openSUSE Leap 16 VM.
# Boots the cloud image under QEMU/KVM, turns SELinux on (enforcing),
# installs the packages of a directory (the tests' among them) and
# OpenShell (test/vm/openshell-version), and runs the installed
# run-tests.sh there: what a test system such as openQA runs.
#
#   test/vm/run-vm.sh <rpm dir>
#
# Needs qemu-system-x86, qemu-utils, cloud-image-utils, curl and /dev/kvm.
# WORK: the working directory (kept; the image is cached there).
# TEST_ARGS: further arguments for run-tests.sh (--selinux-discovery).
# OPENSHELL_PROJECT: the OBS project with openshell-supervisor and
# openshell-sandbox (home:sdrahn:testing); OPENSHELL_REPO: a repository
# URL instead.
# The results come back as $WORK/results.
set -euo pipefail

[ $# = 1 ] || {
	echo "usage: $0 <rpm dir>" >&2
	exit 2
}
rpms=$(realpath "$1")
here=$(cd "$(dirname "$0")" && pwd)
image_url=${IMAGE_URL:-https://download.opensuse.org/distribution/leap/16.0/appliances/Leap-16.0-Minimal-VM.x86_64-Cloud.qcow2}
work=${WORK:-$(mktemp -d)}
mkdir -p "$work"
port=2222
log() { echo "[$(date +%T)] $*"; }

[ -w /dev/kvm ] || {
	echo "/dev/kvm is not available or not writable" >&2
	exit 1
}

log "work directory: $work"
# download_image: the image to $work/base.qcow2, failing fast on a
# stalled or crawling transfer (download.opensuse.org redirects to a
# mirror, and a slow one can take most of the job's time): each of three
# attempts gives up below 1 MB/s for a minute or after 10 minutes, well
# within the job's hour. An image from download.opensuse.org is tried
# there first, then from the origin server (no redirect), then from a
# full mirror. A partial file is never taken for the image; an image
# already there (the CI cache) is used as it is.
download_image() {
	local url urls=("$image_url")
	case $image_url in
	https://download.opensuse.org/*)
		urls+=("https://downloadcontent.opensuse.org/${image_url#https://download.opensuse.org/}"
			"https://ftp.gwdg.de/pub/opensuse/${image_url#https://download.opensuse.org/}") ;;
	*) urls+=("$image_url" "$image_url") ;;
	esac
	for url in "${urls[@]}"; do
		log "downloading $url"
		if curl -fsSL --connect-timeout 30 --speed-limit 1000000 --speed-time 60 --max-time 600 \
			-o "$work/base.qcow2.part" "$url" &&
			qemu-img info "$work/base.qcow2.part" >/dev/null; then
			mv "$work/base.qcow2.part" "$work/base.qcow2"
			log "downloaded $(du -h "$work/base.qcow2" | cut -f1)"
			return 0
		fi
		rm -f "$work/base.qcow2.part"
		sleep 10
	done
	echo "could not download $image_url" >&2
	return 1
}

if [ -f "$work/base.qcow2" ] && qemu-img info "$work/base.qcow2" >/dev/null 2>&1; then
	log "image: $work/base.qcow2 (cached)"
else
	rm -f "$work/base.qcow2"
	download_image
fi
qemu-img create -q -f qcow2 -b "$work/base.qcow2" -F qcow2 "$work/disk.qcow2" 20G

ssh-keygen -q -t ed25519 -N '' -f "$work/key" <<<y >/dev/null
cat >"$work/user-data" <<EOF
#cloud-config
disable_root: false
ssh_pwauth: false
users:
  - name: root
    ssh_authorized_keys:
      - $(cat "$work/key.pub")
growpart:
  mode: auto
  devices: ["/"]
EOF
printf 'instance-id: driver-vmtest\nlocal-hostname: driver-vmtest\n' >"$work/meta-data"
cloud-localds "$work/seed.img" "$work/user-data" "$work/meta-data"

ssh_opts=(-i "$work/key" -p "$port" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null
	-o LogLevel=ERROR -o ConnectTimeout=5 -o ServerAliveInterval=15)
vm() { ssh "${ssh_opts[@]}" root@127.0.0.1 "$@"; }

# boot [restrict]: starts the VM; with restrict, it has no network but
# the forwarded ports (QEMU user networking's restrict=on).
boot() {
	local restrict=${1:+,restrict=on}
	log "booting the VM${1:+ without network}"
	qemu-system-x86_64 -enable-kvm -cpu host -smp 2 -m 4096 \
		-drive file="$work/disk.qcow2",if=virtio \
		-drive file="$work/seed.img",if=virtio,format=raw \
		-nic user,model=virtio-net-pci$restrict,hostfwd=tcp:127.0.0.1:$port-:22 \
		-display none -serial file:"$work/console.log" \
		-pidfile "$work/qemu.pid" -daemonize
}
boot
trap 'kill "$(cat "$work/qemu.pid" 2>/dev/null)" 2>/dev/null || true' EXIT

wait_ssh() {
	local i
	for i in $(seq 120); do
		if vm true 2>/dev/null; then
			log "ssh is up"
			return
		fi
		sleep 5
	done
	echo "the VM did not come up; console:" >&2
	tail -50 "$work/console.log" >&2
	exit 1
}
wait_ssh
vm 'cloud-init status --wait >/dev/null 2>&1 || true'

if [ "$(vm 'getenforce 2>/dev/null || echo Missing')" != Enforcing ]; then
	log "enabling SELinux (state: $(vm 'getenforce 2>/dev/null || echo missing'))"
	vm bash -s <"$here/enable-selinux.sh"
	vm 'systemctl reboot' || true
	sleep 20
	wait_ssh
	# The first boot may relabel and reboot once more.
	for _ in $(seq 30); do
		[ "$(vm 'getenforce 2>/dev/null' || true)" = Enforcing ] && break
		sleep 10
		wait_ssh
	done
fi

# openshell-supervisor and openshell-sandbox, which the driver requires,
# are not in Leap's own repositories: they come from an OBS project
# (OPENSHELL_PROJECT), from its repository for the VM's release. Or set
# OPENSHELL_REPO to a repository's URL.
project=${OPENSHELL_PROJECT:-home:sdrahn:testing}
if [ -z "${OPENSHELL_REPO:-}" ]; then
	base="https://download.opensuse.org/repositories/${project//:/:\/}"
	release=$(vm '. /etc/os-release && echo "$VERSION_ID"')
	for name in "$release" "openSUSE_Leap_$release" "Leap_$release"; do
		if curl -fsSL -m 60 --retry 3 -o /dev/null "$base/$name/repodata/repomd.xml"; then
			OPENSHELL_REPO=$base/$name/
			break
		fi
	done
	if [ -z "${OPENSHELL_REPO:-}" ]; then
		log "no repository of $project for Leap $release; the project has:"
		curl -fsSL -m 60 "$base/" | grep -o 'href="[^"?/][^"]*/"' | sed 's/href="//; s/"$//' >&2 || true
		exit 1
	fi
fi
log "OpenShell's packages from $OPENSHELL_REPO"
vm "zypper -n ar -f -p 105 '$OPENSHELL_REPO' openshell-packages && zypper -n --gpg-auto-import-keys ref openshell-packages" >"$work/repo.log" 2>&1 ||
	{ tail -n 20 "$work/repo.log"; log "adding $OPENSHELL_REPO failed"; exit 1; }

log "installing the packages"
vm 'rm -rf /root/driver-vm && mkdir -p /root/driver-vm/rpms'
scp_opts=(-q -i "$work/key" -P "$port" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR)
scp "${scp_opts[@]}" "$rpms"/*.rpm root@127.0.0.1:/root/driver-vm/rpms/
# The tests come from their package, as on a test system.
tests=/usr/libexec/openshell-driver-systemd/tests
vm 'zypper -n --no-gpg-checks --gpg-auto-import-keys in --no-recommends /root/driver-vm/rpms/*.rpm' >"$work/install.log" 2>&1 ||
	{ tail -n 40 "$work/install.log"; log "installing the packages failed"; exit 1; }

vm "$tests/openshell-setup.sh" 2>&1 | tee "$work/openshell-setup.log"
[ "${PIPESTATUS[0]}" = 0 ] || { log "setting up OpenShell failed"; exit 1; }
set +e
vm "$tests/run-tests.sh --disposable --results /root/driver-vm/results ${TEST_ARGS:-}" 2>&1 | tee "$work/test.log"
status=${PIPESTATUS[0]}
set -e
rm -rf "$work/results"
scp -r "${scp_opts[@]}" root@127.0.0.1:/root/driver-vm/results "$work/" || true
log "the tests exited with $status"
exit "$status"
