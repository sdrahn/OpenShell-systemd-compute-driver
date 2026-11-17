# SUSE's openQA tests
#
# SPDX-FileCopyrightText: Copyright (c) 2026 Daniel Rahn
# SPDX-License-Identifier: FSFAP
#
# Package: openshell-driver-systemd openshell-driver-systemd-tests openshell
# Summary: Functional tests of the OpenShell systemd compute driver
#          (package openshell-driver-systemd-tests): sandboxes as systemd
#          units through a stock OpenShell gateway, what a workload, a
#          supervisor and the helper must not reach, the SELinux domains
#          and the Landlock fences. Each check becomes a result of its
#          own (JUnit).
# Maintainer: the driver's maintainers
#
# Settings:
#   OPENSHELL_DRIVER_SYSTEMD_OFFLINE=1   leave out the checks that reach a
#                                        host on the Internet (they become
#                                        SKIP)

use Mojo::Base 'consoletest';
use testapi;
use serial_terminal 'select_serial_terminal';
use utils qw(zypper_call);

my $tests = '/usr/libexec/openshell-driver-systemd/tests';
my $results = '/var/tmp/openshell-driver-systemd-tests';

sub run {
    select_serial_terminal;
    # The tests check the SELinux domains, so the policy module is part of
    # what is tested. The tests' package requires the driver and the tools.
    zypper_call('in openshell-driver-systemd-tests openshell-driver-systemd-selinux');
    assert_script_run("mkdir -p $results");
    # OpenShell from the repositories, its gateway as a service of the user
    # "osh", registered for root too.
    assert_script_run("$tests/openshell-setup.sh --from-repositories > $results/setup.log 2>&1", timeout => 900);
    my $offline = get_var('OPENSHELL_DRIVER_SYSTEMD_OFFLINE') ? ' --offline' : '';
    # Exit status 0: all passed; 1: a check failed (the JUnit results say
    # which); 2: wrong usage.
    my $rc = script_run("$tests/run-tests.sh --disposable --results $results$offline > $results/run.log 2>&1", timeout => 2700);
    upload_logs("$results/setup.log");
    upload_logs("$results/run.log");
    die "run-tests.sh did not run (exit status $rc)" if $rc == 2 || script_run("test -s $results/results.xml") != 0;
    upload_logs("$results/results.tap");
    parse_extra_log(XUnit => "$results/results.xml");
}

sub post_fail_hook {
    my ($self) = @_;
    select_serial_terminal;
    upload_logs("$results/setup.log", failok => 1);
    upload_logs("$results/run.log", failok => 1);
    # What run-tests.sh keeps: the denials and the units' journals.
    for my $file (qw(avc.txt avc-unexpected.txt helper.journal units.journal user.journal)) {
        upload_logs("$results/$file", failok => 1);
    }
    script_run("journalctl --no-pager -o short-precise -b > $results/journal.txt 2>&1");
    upload_logs("$results/journal.txt", failok => 1);
    $self->SUPER::post_fail_hook;
}

sub test_flags {
    return {fatal => 0};
}

1;
