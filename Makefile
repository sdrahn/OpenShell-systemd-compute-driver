# SPDX-FileCopyrightText: Copyright (c) 2026 Daniel Rahn
# SPDX-License-Identifier: Apache-2.0

SELINUX_DEVEL ?= /usr/share/selinux/devel/Makefile

.PHONY: all build check fmt-check lint test selinux clean

all: build

build:
	cargo build --release --locked

# What a pull request must pass before it is pushed. check-docs: the
# guide's structure, as mcp-docs reads it.
check: fmt-check lint test
	tools/changelog check
	tools/openshell-version >/dev/null
	tools/check-tree
	tools/check-docs
	test/tools/check-release-test.sh

fmt-check:
	cargo fmt --all -- --check

lint:
	cargo clippy --locked --all-targets -- -D warnings

test:
	cargo test --locked

# Needs selinux-policy-devel.
selinux: selinux/openshell_systemd.pp

selinux/openshell_systemd.pp: selinux/openshell_systemd.te selinux/openshell_systemd.fc
	$(MAKE) -C selinux -f $(SELINUX_DEVEL) openshell_systemd.pp

clean:
	cargo clean
	rm -rf selinux/*.pp selinux/tmp
