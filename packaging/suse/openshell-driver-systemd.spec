#
# spec file for package openshell-driver-systemd
#
# SPDX-FileCopyrightText: Copyright (c) 2026 Daniel Rahn
# SPDX-License-Identifier: Apache-2.0
#

# The OpenShell release the driver is built against. The sandboxes run
# openshell-supervisor and openshell-sandbox of the same release, from
# their own packages (tools/openshell-version keeps this line, Cargo.toml,
# _service and the tests in step).
%define openshell_version 0.1.10
%define selinuxtype targeted
%define modulename openshell_systemd

Name:           openshell-driver-systemd
Version:        0.4.0
Release:        0
Summary:        OpenShell compute driver that runs sandboxes as systemd units
License:        Apache-2.0
URL:            https://github.com/sdrahn/OpenShell-systemd-compute-driver
Source0:        %{name}-%{version}.tar.gz
Source1:        openshell-%{openshell_version}.tar.gz
# The driver's crates, as OBS's cargo_vendor service makes them
# (_service, packaging/suse/README.md): Cargo.lock, .cargo/config.toml and
# the vendor directory. OpenShell's source is for its crates the driver
# uses, which are built from that tree.
Source2:        vendor.tar.zst
BuildRequires:  cargo >= 1.94
# The C compiler and linker: aws-lc-sys (OpenShell's crypto, through
# aws-lc-rs) builds AWS-LC with cc, from bindings it ships for x86_64 and
# aarch64, so neither CMake nor clang is needed.
BuildRequires:  gcc
# The policy module (selinux/).
BuildRequires:  make
BuildRequires:  pkgconfig
# The macros below (%%sysusers_requires, the service macros) add this
# requirement in the build root. The build service reads the spec
# without them, so it is named here for it to install.
BuildRequires:  pkgconfig(systemd)
# OpenShell's build would otherwise run a protoc binary from a crate.
BuildRequires:  protobuf-devel
BuildRequires:  sysuser-tools
BuildRequires:  systemd-rpm-macros
BuildRequires:  zstd
Requires:       systemd >= 257
# The stock programs a sandbox runs, of the release the driver is built
# against: the supervisor speaks the gateway's protocol of that release.
Requires:       openshell-sandbox = %{openshell_version}
Requires:       openshell-supervisor = %{openshell_version}
Requires:       (%{name}-selinux if selinux-policy-%{selinuxtype})
ExclusiveArch:  x86_64 aarch64
%sysusers_requires
%{?systemd_ordering}

%description
An external compute driver for the OpenShell gateway. It runs each
sandbox as systemd units on the gateway's host, without a container
runtime or a VM: the stock supervisor on the host's network and the
stock openshell-sandbox in a network namespace with loopback only, both
as an unprivileged user of the sandbox's own. The workload uses the host's file system
read-only.

The package holds the driver, its privileged helper and the unit
templates. openshell-supervisor and openshell-sandbox of OpenShell
%{openshell_version} come with their own packages.

%package selinux
Summary:        SELinux policy module for openshell-driver-systemd
BuildRequires:  selinux-policy-devel
Requires:       %{name} = %{version}
Requires:       selinux-policy-%{selinuxtype}
Requires(post): selinux-policy-%{selinuxtype}
BuildArch:      noarch
# %%selinux_requires asks for the build's selinux-policy version and
# release, so a build against a rebuild or maintenance update of the same
# policy version cannot be installed on a host without that update (a
# Package Hub build for SLES 16.1 asked for selinux-policy's release
# 160150.1.13; the host had 160100.1.4 of the same version). The module
# needs the policy version only: drop the release.
%{?_selinux_policy_version:%global _selinux_policy_version %(echo '%{_selinux_policy_version}' | cut -d- -f1)}
%{?selinux_requires}

%description selinux
SELinux domains for the sandboxes of openshell-driver-systemd: one for a
workload, one for its supervisor. A workload reaches its supervisor's
control socket and no other service's.

%package docs
Summary:        Guide of openshell-driver-systemd, also for mcp-docs
Group:          Documentation/Other
BuildArch:      noarch
# Installed with mcp-docs, it serves the guide to agents.
Supplements:    (%{name} and mcp-docs)

%description docs
The guide of openshell-driver-systemd in Markdown, for administrators
and for those who work in a sandbox: installing and setting up, what a
sandbox is and can do, network policy, the security model, operation,
troubleshooting and reference, and the security review, in
/usr/share/openshell-driver-systemd/docs. With a collection file for
mcp-docs, which then serves them as the collection
openshell-driver-systemd to the agents and assistants on the host.

%package tests
Summary:        Functional tests of openshell-driver-systemd
Requires:       %{name} = %{version}
Requires:       audit
Requires:       curl
Requires:       gcc
Requires:       git-core
Requires:       gzip
Requires:       iproute2
Requires:       make
# A server certificate of a private CA, for the test of the host's trust store.
Requires:       openssl
Requires:       policycoreutils
Requires:       procps
Requires:       python3
Requires:       tar
Requires:       util-linux
Recommends:     %{name}-selinux = %{version}
Recommends:     %{name}-docs = %{version}
BuildArch:      noarch

%description tests
The functional tests of openshell-driver-systemd, for openQA and other
systems that test a product with the packages of its repositories:
sandboxes are created, used, stopped, started and deleted through a
stock OpenShell gateway, and what a workload, a supervisor and the
helper must not reach is tried. Each check becomes a result of its own
(TAP and JUnit). The tests change the system they run on and refuse to
start without --disposable.

%prep
%setup -q -a 1 -a 2
# OpenShell's crates from its source tree, not from the vendored copies:
# their build reads the proto files of the tree, which a vendored crate
# does not have next to it.
{
    echo
    echo '[patch."https://github.com/NVIDIA/OpenShell"]'
    awk '/^name = "openshell-/ { gsub(/"/, "", $3); name = $3 }
         /^source = "git\+https:\/\/github.com\/NVIDIA\/OpenShell/ { print name }' Cargo.lock |
    while read -r crate; do
        echo "$crate = { path = \"openshell-%{openshell_version}/crates/$crate\" }"
    done
} >>.cargo/config.toml
tail -n 12 .cargo/config.toml

%build
export CARGO_HOME=$PWD/.cargo
export PROTOC=%{_bindir}/protoc
export PROTOC_INCLUDE=%{_includedir}
# Not --locked: with the crates above taken from the tree, Cargo rewrites
# their entries in the lock file. Every other crate stays as locked, and
# nothing can be fetched.
cargo build --release --offline
%sysusers_generate_pre packaging/suse/%{name}.sysusers %{name} %{name}.conf
make -f %{_datadir}/selinux/devel/Makefile -C selinux %{modulename}.pp

%check
export CARGO_HOME=$PWD/.cargo
export PROTOC=%{_bindir}/protoc
export PROTOC_INCLUDE=%{_includedir}
cargo test --release --offline

%install
# The unit templates name the programs by these paths (and
# openshell-supervisor and openshell-sandbox by %{_bindir}, where their
# packages put them).
test "%{_libexecdir}" = /usr/libexec
install -D -m 0755 target/release/openshell-driver-systemd %{buildroot}%{_bindir}/openshell-driver-systemd
install -D -m 0755 target/release/openshell-systemd-helper %{buildroot}%{_libexecdir}/openshell/openshell-systemd-helper
install -D -m 0755 target/release/openshell-supervisor-fence %{buildroot}%{_libexecdir}/openshell/openshell-supervisor-fence
install -d %{buildroot}%{_unitdir} %{buildroot}%{_userunitdir}
# The mount points the sandbox units bind to (the paths the stock
# supervisor and openshell-sandbox use in a container). The package makes
# them: on a transactional system the root file system is read-only once
# the snapshot is in use, and the helper could not make them there.
install -d %{buildroot}/.openshell/supervisor %{buildroot}/.openshell/channel %{buildroot}/sandbox
install -m 0644 systemd/*.service %{buildroot}%{_unitdir}/
install -m 0644 systemd/user/*.service %{buildroot}%{_userunitdir}/
install -D -m 0644 packaging/suse/%{name}.sysusers %{buildroot}%{_sysusersdir}/%{name}.conf
install -D -m 0644 systemd/tmpfiles.d/%{name}.conf %{buildroot}%{_tmpfilesdir}/%{name}.conf
install -d %{buildroot}%{_libexecdir}/%{name}/tests
install -m 0755 test/vm/run-tests.sh test/vm/openshell-setup.sh %{buildroot}%{_libexecdir}/%{name}/tests/
install -m 0644 test/vm/openshell-version %{buildroot}%{_libexecdir}/%{name}/tests/
# Not %%doc: a system installed without documentation would leave the
# guide out, and mcp-docs is to serve it there too.
install -d %{buildroot}%{_datadir}/%{name}/docs/user-guide
install -m 0644 docs/user-guide/*.md %{buildroot}%{_datadir}/%{name}/docs/user-guide/
install -m 0644 docs/security-review-*.md %{buildroot}%{_datadir}/%{name}/docs/
install -D -m 0644 packaging/mcp-docs/%{name}.yaml %{buildroot}%{_datadir}/mcp-docs/collections.d/%{name}.yaml
install -d %{buildroot}%{_datadir}/selinux/packages/%{selinuxtype}
bzip2 -9 -c selinux/%{modulename}.pp >%{buildroot}%{_datadir}/selinux/packages/%{selinuxtype}/%{modulename}.pp.bz2

%pre -f %{name}.pre
%service_add_pre openshell-systemd-helper.service

%post
%tmpfiles_create %{_tmpfilesdir}/%{name}.conf
%service_add_post openshell-systemd-helper.service
%systemd_user_post openshell-driver-systemd.service

%preun
%service_del_preun openshell-systemd-helper.service
%systemd_user_preun openshell-driver-systemd.service

%postun
%service_del_postun openshell-systemd-helper.service
%systemd_user_postun openshell-driver-systemd.service

%pre selinux
%selinux_relabel_pre -s %{selinuxtype}

%post selinux
%selinux_modules_install -s %{selinuxtype} %{_datadir}/selinux/packages/%{selinuxtype}/%{modulename}.pp.bz2

%postun selinux
if [ $1 -eq 0 ]; then
    %selinux_modules_uninstall -s %{selinuxtype} %{modulename}
fi

%posttrans selinux
%selinux_relabel_post -s %{selinuxtype}

%files
%license LICENSE
%doc README.md CHANGELOG.md
%{_bindir}/openshell-driver-systemd
%dir %{_libexecdir}/openshell
%{_libexecdir}/openshell/openshell-systemd-helper
%{_libexecdir}/openshell/openshell-supervisor-fence
%{_unitdir}/openshell-systemd-helper.service
%{_unitdir}/openshell-sandbox-netns@.service
%{_unitdir}/openshell-sandbox-supervisor@.service
%{_unitdir}/openshell-sandbox-workload@.service
%{_userunitdir}/openshell-driver-systemd.service
%{_sysusersdir}/%{name}.conf
%{_tmpfilesdir}/%{name}.conf
%dir /.openshell
%dir /.openshell/supervisor
%dir /.openshell/channel
%dir /sandbox

%files docs
%license LICENSE
%dir %{_datadir}/%{name}
%{_datadir}/%{name}/docs
# mcp-docs owns these directories too; whichever package comes first
# creates them.
%dir %{_datadir}/mcp-docs
%dir %{_datadir}/mcp-docs/collections.d
%{_datadir}/mcp-docs/collections.d/%{name}.yaml

%files tests
%dir %{_libexecdir}/%{name}
%dir %{_libexecdir}/%{name}/tests
%{_libexecdir}/%{name}/tests/run-tests.sh
%{_libexecdir}/%{name}/tests/openshell-setup.sh
%{_libexecdir}/%{name}/tests/openshell-version

%files selinux
%{_datadir}/selinux/packages/%{selinuxtype}/%{modulename}.pp.bz2

%changelog
