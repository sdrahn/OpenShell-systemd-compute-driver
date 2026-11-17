---
title: "1. What the driver is and when to use it"
description: The parts of the OpenShell systemd compute driver (driver, helper, three units per sandbox, the stock supervisor and openshell-sandbox), which part enforces which rule, how it differs from OpenShell's container drivers, what it does not support, and the terms the guide uses.
tags: [overview, concepts]
---

# 1. What the driver is and when to use it

`openshell-driver-systemd` is a compute driver for
[OpenShell](https://github.com/NVIDIA/OpenShell): the part of an
OpenShell gateway that makes, starts, stops and removes sandboxes. This
one makes a sandbox out of systemd units on the gateway's host. This
chapter names the parts and the terms the other chapters use.

## What it is for {#when}

Use it on a Linux host where

- the workload should use **the host's own installed tools**: the
  compiler, the package database, the administration commands of that
  very system, and
- **systemd units and the host's security policy** (SELinux, Landlock)
  are how services are run and confined there, and a container runtime
  or a VM is not wanted for it.

A sandbox of this driver has no image. Its root file system is the
host's, read-only. That is the point of the driver and its main
difference from OpenShell's container drivers (Podman, Docker,
Kubernetes) and its VM driver: use one of those when a workload needs
its own file system, its own packages, or a harder wall than a host's
user and namespaces.

It is an external driver: a project of its own, which a stock gateway
uses through OpenShell's interface for drivers on a Unix socket. It was
built and tested against one OpenShell release, named in
[chapter 8](08-reference.md#packages).

## The parts {#parts}

| Part | Runs as | What it does |
|---|---|---|
| the OpenShell gateway | its own user | OpenShell's control plane: its API, its policies, its sandboxes' credentials. Not part of this driver |
| `openshell-driver-systemd`, the driver | the gateway's user, as a service of that user | answers the gateway on a Unix socket; prepares a sandbox's launch files; has no privilege |
| `openshell-systemd-helper`, the helper | root, as a system service | writes a sandbox's files and starts, stops and reads the state of its units; serves one user |
| three units per sandbox | see below | the sandbox itself |
| `openshell-supervisor` | `openshell-sandbox` | OpenShell's stock supervisor: holds the sandbox's credentials, talks to the gateway, mediates the workload's network |
| `openshell-sandbox` | `openshell-sandbox` | OpenShell's stock workload launcher: applies OpenShell's Landlock and seccomp policy and starts the agent |
| `openshell-supervisor-fence` | `openshell-sandbox` | a launcher of this driver: puts a Landlock fence around the supervisor, then becomes it |

Two programs, because the gateway accepts a driver socket of its own
user only, and that user cannot start system units. The helper is the
privilege boundary; [chapter 9](09-helper-protocol.md) says what crosses
it.

## What a sandbox is {#sandbox}

Three instances of unit templates, named by the sandbox's ID (a UUID):

| Unit | Runs | As | Network |
|---|---|---|---|
| `openshell-sandbox-netns@ID.service` | makes and holds the sandbox's network namespace | root, one capability | a namespace with loopback only |
| `openshell-sandbox-supervisor@ID.service` | `openshell-supervisor` | `openshell-sandbox` | the host's |
| `openshell-sandbox-workload@ID.service` | `openshell-sandbox`, which starts the agent | `openshell-sandbox` | the namespace of the first unit |

The workload has no network interface but loopback. Whatever it sends
out goes over a Unix socket to its supervisor, which decides by the
sandbox's policy ([chapter 4](04-network-policy.md)).

## Who enforces what {#enforcement}

| Rule | Enforced by |
|---|---|
| the workload has no route to the network | the kernel: a network namespace (`PrivateNetwork=`) |
| which hosts a workload reaches, with which program | OpenShell's supervisor, by the sandbox's policy |
| which files the workload reads and writes | the host's file modes for the user `openshell-sandbox`; OpenShell's Landlock policy, set by `openshell-sandbox`; the unit's mount namespace; SELinux |
| which system calls the workload makes | OpenShell's seccomp filter, set by `openshell-sandbox` |
| the workload gains no privilege | systemd: no capabilities, `NoNewPrivileges=yes` |
| the workload sees and signals only its own processes | the kernel: a process ID namespace (`PrivatePIDs=`) |
| one sandbox cannot use another's files | SELinux categories; for supervisors also a Landlock fence and the unit's mount namespace |
| the helper does only what it is for | the protocol; a Landlock fence; an SELinux domain; five capabilities |

None of this is held only by code in the driver. The driver has no
privilege to hold it with. [Chapter 5](05-security.md) goes through the
layers and their limits.

## What it does not support {#not-supported}

- **Images.** A request that names one is refused
  ([chapter 3](03-sandboxes.md#no-image)).
- **CPU, memory, disk and GPU requests** of a sandbox. `TasksMax=2048`
  in the workload's template is the only limit; an administrator sets
  others for all sandboxes with drop-ins
  ([chapter 6](06-operation.md#limits)).
- **Suspend and resume**, and authenticating a sandbox's credentials on
  the gateway's behalf.
- **A workload user per sandbox.** Every sandbox runs as
  `openshell-sandbox`; a policy that asks for another user or group is
  refused.
- **Hosts other than SLES 16 and openSUSE Leap 16.** It needs systemd
  257 and was tested on Leap 16.0 only.

## Terms {#terms}

| Term | Meaning |
|---|---|
| the gateway | the OpenShell gateway |
| the gateway's user | the host user the gateway runs as; the driver runs as the same user, and the helper serves that user only |
| workload | what runs in a sandbox: `openshell-sandbox` and everything it starts, the agent among it |
| supervisor | `openshell-supervisor` of one sandbox |
| the workloads' user | `openshell-sandbox`, the one user every supervisor and workload runs as |
| launch | one start of a sandbox: the helper writes its files anew, all but the workspace |
| launch files | what a supervisor and a workload are started with: bootstrap file, TLS identity, token, runtime descriptor |
| workspace | the sandbox's own directory, `/sandbox` inside it; kept from launch to launch |
| state directory | `/var/lib/openshell/systemd`, where the helper keeps each sandbox's directory |
| fence | a Landlock rule set a program puts around itself; it binds root too |
| categories | a pair of SELinux MCS categories (`c7,c512`) that a sandbox's processes and files carry |
