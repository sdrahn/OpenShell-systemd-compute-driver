---
title: "4. Network access and policy"
description: How a workload of the OpenShell systemd compute driver reaches the network (only through its supervisor), a worked example of a sandbox policy that lets one program reach one host, how programs are told apart, the file system part of a policy, and who decides what.
tags: [user, network, policy, example]
---

# 4. Network access and policy

A workload has no network of its own. Every connection it makes to the
outside is made for it by its supervisor, which allows or refuses it by
the sandbox's policy. The policy language is OpenShell's; this chapter
shows the part of it that matters here, with an example that the
functional tests run.

## How a connection leaves a sandbox {#path}

1. The workload's network namespace has loopback only. A direct
   connection (`connect` to an address on the Internet) fails: there is
   no route.
2. `openshell-sandbox` serves the workload a name service on loopback
   (`/etc/resolv.conf` in the sandbox names `127.0.0.53`), and what the
   workload sends out goes over the control socket to the supervisor.
3. The supervisor, on the host's network, looks at the destination and
   at **which program** of the workload is asking, and decides by the
   policy. For an endpoint with `protocol: rest` it also reads the
   request: it ends the TLS connection itself, with a certificate
   authority that the workload is given to trust.
4. What the policy does not name is refused.

Proxy, name service, certificate authority and the decision are all the
stock supervisor's. The driver adds the namespace and nothing else, so
OpenShell's documentation on policies applies unchanged.

What a sandbox **without** a policy of its own may reach is OpenShell's
default policy. The driver does not change it.

## A worked example: one program, one host {#example}

A sandbox in which `curl` may read from `example.com` and nothing else
may go anywhere:

```yaml
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
```

```shell
openshell sandbox create --name net --policy policy.yaml --detach -- sleep 3600
openshell sandbox exec net -- curl -sS -o /dev/null -w '%{http_code}\n' https://example.com/
# 200
openshell sandbox exec net -- curl -sS -m 20 https://www.opensuse.org/
# fails: the policy does not name this host
openshell sandbox exec net -- sh -c 'cp /usr/bin/curl /sandbox/other-curl && /sandbox/other-curl -sS -m 20 https://example.com/'
# fails: the policy does not name this program
```

`curl` is the host's `/usr/bin/curl`: it has to be installed on the
host.

## How programs are told apart {#binaries}

`binaries` names programs by path, and the supervisor checks which
program a request comes from. The tests hold this:

| Program | Result |
|---|---|
| `/usr/bin/curl`, named in the policy | reaches the host |
| a copy of `curl` in `/sandbox` that the policy does not name | refused |
| a copy in `/sandbox` or `/tmp` that the policy names by its path | reaches the host |

So "the host is allowed, but my request is refused" usually means that
another program is asking than the one the policy names: an
interpreter (`python3`) instead of `curl`, a tool from a virtual
environment in the workspace instead of the host's.

Naming a path in `/sandbox` or `/tmp` allows whatever the workload puts
there. Name the host's programs where that is enough.

## The file system part of a policy {#filesystem}

`filesystem_policy` is what `openshell-sandbox` turns into the
workload's Landlock rules. The lists in the example are OpenShell's
defaults. Two things are particular to this driver:

- The paths are **the host's**. `/usr` is the host's `/usr`. `/app`
  exists only if the host has it.
- The policy can narrow what the unit gives, not widen it. The host is
  mounted read-only whatever `read_write` says, and `/home`, `/root`
  and the host's `/run` are not in the sandbox at all
  ([chapter 3](03-sandboxes.md#files)).

`landlock: compatibility: best_effort` lets a sandbox start on a kernel
that cannot hold every rule. This is OpenShell's setting for the
workload, not the one for the helper and the supervisor in
[chapter 5](05-security.md#landlock).

## A host with a private certificate authority {#private-ca}

For an endpoint with `protocol: rest` (or `mcp`) the supervisor ends
the workload's TLS connection and opens its own to the host, which it
verifies with the CA certificates of the host it runs on: on SLES and
openSUSE `/etc/ssl/ca-bundle.pem`. A host whose certificate a private
CA signed is reached once that CA is in the host's trust store:

```shell
cp private-ca.pem /etc/pki/trust/anchors/
update-ca-certificates
```

A sandbox's supervisor reads it when the sandbox starts; restart the
sandboxes that run.

OpenShell's own setting for this, `additional_ca_bundle` in
`[openshell.supervisor.network]` of `gateway.toml`, is for its
container drivers. With an external compute driver such as this one
OpenShell (0.1.8 to 0.1.10) refuses the setting, and the gateway does
not start.

## Who decides what {#who}

| Decision | Made by | Where |
|---|---|---|
| which hosts and programs a sandbox may use | whoever creates the sandbox, within what the gateway allows | the sandbox's policy |
| what the gateway accepts as a policy | the gateway's administrator | OpenShell's configuration |
| which tools exist in a sandbox | the host's administrator | installed packages |
| what no policy can give (the list in [chapter 3](03-sandboxes.md#cannot)) | the unit templates and the SELinux module | [chapter 5](05-security.md) |
| limits on memory and CPU | the host's administrator | drop-ins ([chapter 6](06-operation.md#limits)) |

A workload cannot change its own policy: the policy is held by the
gateway and enforced by the supervisor, and the workload reaches
neither's files.
