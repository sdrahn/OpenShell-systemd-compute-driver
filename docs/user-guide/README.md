---
title: OpenShell systemd compute driver guide
description: Index of the guide to openshell-driver-systemd, the OpenShell compute driver that runs sandboxes as systemd units on SLES 16 and openSUSE Leap 16 - installing, operating and securing it for administrators, and what a sandbox is for the people and agents that work in one - with the common questions mapped to the sections that answer them.
tags: [index, openshell, systemd, user guide]
---

# OpenShell systemd compute driver guide

`openshell-driver-systemd` lets a stock OpenShell gateway run its
sandboxes as systemd units on the gateway's own host, without a
container runtime or a VM. Chapters 1, 2 and 5 to 8 are for the
administrator of that host; chapters 3 and 4 are for whoever works in a
sandbox or writes its policy. "The gateway" is always the OpenShell
gateway.

## Chapters {#chapters}

| Chapter | For |
|---|---|
| [1. What the driver is and when to use it](01-overview.md) | the parts, who enforces what, how it differs from the container drivers, what it does not do, the terms |
| [2. Installing and setting up](02-installation.md) | requirements, packages, the three setup steps, checking, upgrading, removing |
| [3. Working in a sandbox](03-sandboxes.md) | creating one, what a workload sees and can do, the workspace, stop and start, what is different from a container |
| [4. Network access and policy](04-network-policy.md) | how a workload reaches the network, a worked example of a policy, who decides it |
| [5. The security model](05-security.md) | the threat model, each layer and what it holds, SELinux, Landlock, the known limits |
| [6. Operating the driver](06-operation.md) | the units and files of a sandbox, logs, limits with drop-ins, restarts, reboots, backups |
| [7. Troubleshooting](07-troubleshooting.md) | symptoms, causes and fixes; the error messages |
| [8. Reference](08-reference.md) | programs and their settings, units, files, users, SELinux types, packages |
| [9. The helper protocol](09-helper-protocol.md) | what the driver asks of the privileged helper, and how |
| [10. What the tests hold](10-tested.md) | which statements of this guide a functional test checks, and which none does |

The review before the first release is next to this guide:
[security-review-0.1.md](../security-review-0.1.md).

## Common questions {#questions}

| Question | Section |
|---|---|
| Is this driver the right one for my host? | [01-overview.md#when](01-overview.md#when) |
| What does it not support? | [01-overview.md#not-supported](01-overview.md#not-supported) |
| Which packages do I install? | [02-installation.md#packages](02-installation.md#packages) |
| How do I connect the gateway to the driver? | [02-installation.md#gateway](02-installation.md#gateway) |
| How do I check that the installation works? | [02-installation.md#check](02-installation.md#check) |
| How do I remove it again? | [02-installation.md#remove](02-installation.md#remove) |
| Why is my image refused? | [03-sandboxes.md#no-image](03-sandboxes.md#no-image) |
| What can an agent read and write in a sandbox? | [03-sandboxes.md#files](03-sandboxes.md#files) |
| Where are a sandbox's files on the host? | [03-sandboxes.md#workspace](03-sandboxes.md#workspace) |
| Why does `systemctl` or `journalctl` not work in a sandbox? | [03-sandboxes.md#cannot](03-sandboxes.md#cannot) |
| How do I let a program reach one host? | [04-network-policy.md#example](04-network-policy.md#example) |
| Why is a request refused although the host is allowed? | [04-network-policy.md#binaries](04-network-policy.md#binaries) |
| Can one sandbox reach another's files? | [05-security.md#between-sandboxes](05-security.md#between-sandboxes) |
| What is weaker without SELinux? | [05-security.md#without-selinux](05-security.md#without-selinux) |
| What are the known limits? | [05-security.md#limits](05-security.md#limits) |
| How do I limit a sandbox's memory or CPU? | [06-operation.md#limits](06-operation.md#limits) |
| Where are the logs? | [06-operation.md#logs](06-operation.md#logs) |
| What happens at a reboot? | [06-operation.md#reboot](06-operation.md#reboot) |
| A sandbox does not become ready. What now? | [07-troubleshooting.md#not-ready](07-troubleshooting.md#not-ready) |
| The helper does not start. | [07-troubleshooting.md#helper](07-troubleshooting.md#helper) |
| What does this error message mean? | [07-troubleshooting.md#messages](07-troubleshooting.md#messages) |
| Which settings does the helper have? | [08-reference.md#helper](08-reference.md#helper) |
| Which settings does the driver have? | [08-reference.md#driver](08-reference.md#driver) |
| Which SELinux types are there? | [08-reference.md#selinux](08-reference.md#selinux) |
| Was this behaviour tested? | [10-tested.md](10-tested.md) |
