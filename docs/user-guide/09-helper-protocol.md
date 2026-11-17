---
title: "9. The helper protocol"
description: Reference of the protocol between openshell-driver-systemd and its privileged helper openshell-systemd-helper - one JSON request and response per connection on a Unix socket, the seven requests, the fields of a launch, the answers, the unit states, and the rules that make it the privilege boundary.
tags: [reference, protocol, helper, security]
---

# 9. The helper protocol

The helper is the only part of the driver that runs as root, and this
protocol is everything that can be asked of it. It is internal: the
driver and the helper of one package version speak it to each other,
and it may change between versions. It is written down here because it
is the privilege boundary an administrator or reviewer wants to judge.
The definition is `src/protocol.rs`.

## The rules that make it a boundary {#rules}

- A request carries **file contents and a sandbox ID**. It never
  carries a path, a unit name, a command, a user or a mode. The helper
  decides where each content goes and whose it is.
- The sandbox ID must be a UUID in its canonical form (lower case,
  hyphenated, 36 characters). It is checked before anything else, for
  every request, and is the only field that reaches a path or a unit
  name.
- The supervisor's environment may hold only names that begin with
  `OPENSHELL_` and not with `OPENSHELL_SYSTEMD_`, and no value with a
  control character.
- Only one user is served, checked from the socket's peer credentials
  (`SO_PEERCRED`) for every connection. Root is not served either.
- A sandbox's name and OpenShell workspace are stored and returned;
  they are never used as a path.
- A request cannot name a user. The helper chooses a sandbox's user
  from its own range, never root and never an account of the host, and
  reads it back only from a file of root's.

## Transport {#transport}

A stream connection to the Unix socket
`/run/openshell-systemd-helper/helper.sock` (mode 0600, owned by the
allowed user).

1. The client writes one JSON document, the request, and closes its
   sending half.
2. The helper writes one JSON document, the answer, and closes.

A request has 30 seconds to arrive. A message is at most 4 MiB in
either direction. Requests are served one at a time. A connection of
another user is closed without an answer.

By hand, as the gateway's user:

```shell
python3 -c '
import socket, sys
s = socket.socket(socket.AF_UNIX)
s.connect("/run/openshell-systemd-helper/helper.sock")
s.sendall(sys.argv[1].encode()); s.shutdown(socket.SHUT_WR)
print(s.recv(65536).decode())' '"describe"'
```

## Requests {#requests}

| Request | JSON | Does | Good answer |
|---|---|---|---|
| describe | `"describe"` | nothing | `described` |
| reserve | `{"reserve": {"sandbox_id": "ID"}}` | gives the sandbox a user: the one it has, or the lowest free ID of the range. Makes the sandbox's directory for it | `reserved` |
| provision | `{"provision": {…}}` | stops the sandbox's units; writes its launch files anew, all but the workspace, as the sandbox's user's; labels them. Refused for a sandbox without a reserved user | `done` |
| start | `{"start": {"sandbox_id": "ID"}}` | writes the user's name and the drop-ins (user, SELinux level) where they are missing; starts the sandbox's units | `done`, `not_found` |
| stop | `{"stop": {"sandbox_id": "ID"}}` | stops the units; keeps the files | `done`, `not_found` |
| delete | `{"delete": {"sandbox_id": "ID"}}` | stops the units; removes the sandbox's directory, the workspace too, its drop-ins and its user's name | `done`, `not_found` |
| status | `{"status": {"sandbox_id": "ID"}}` | reads the units' state | `status`, `not_found` |
| list | `"list"` | reads every sandbox's state | `listed` |

## The fields of provision {#provision}

All are strings but `environment`.

| Field | Becomes |
|---|---|
| `sandbox_id` | the directory and the unit instance |
| `name`, `workspace` | the sandbox's name and OpenShell workspace, in `meta.json` |
| `bootstrap` | `openshell-sandbox`'s bootstrap file, in `channel/sandbox/` |
| `server_certificate`, `server_key` | the sandbox's TLS identity, in `channel/sandbox/` |
| `runtime_descriptor` | the supervisor's runtime descriptor, in `supervisor/` |
| `auth_bundle` | the supervisor's credentials for this launch, in `supervisor/` |
| `sandbox_token` | the supervisor's gateway token, `etc/sandbox.jwt`. Empty: the token of the launch before is kept, since the gateway sends it only when it creates a sandbox |
| `gateway_ca` | the CA the supervisor verifies the gateway with, `etc/ca.crt`; empty for none |
| `environment` | an object of names and values: the supervisor's `supervisor.env` |

Owners and modes are in [chapter 6](06-operation.md#files). The
credentials among these fields are not logged by either side.

## Answers {#answers}

| Answer | JSON |
|---|---|
| done | `"done"` |
| described | `{"described": {"version": "0.1.0", "first_uid": 70000, "last_uid": 79999}}`: the helper's version and the range its sandboxes' users are from |
| reserved | `{"reserved": {"uid": 70000, "gid": 70000}}`: the host user and group of this sandbox |
| status | `{"status": RECORD}` |
| listed | `{"listed": {"sandboxes": [RECORD, …]}}` |
| not found | `"not_found"`: the helper holds no files for this sandbox |
| error | `{"error": {"message": "…"}}` ([chapter 7](07-troubleshooting.md#messages)) |

A record:

```json
{
  "sandbox_id": "d6ac9d5f-60aa-4fcb-b050-9a0a4b134205",
  "name": "work",
  "workspace": "default",
  "state": "active",
  "detail": "supervisor active, workload active"
}
```

## Unit states {#states}

`state` sums up the supervisor's and the workload's unit; `detail` is
what systemd says of each.

| State | Means | The gateway is told (`Ready` condition) |
|---|---|---|
| `active` | both units are active | true |
| `transitioning` | a unit is starting or stopping | false, starting |
| `inactive` | neither is active and none failed | false, stopped |
| `failed` | a unit failed, or one runs without the other | false, exited; shown as phase `Error` |

The driver asks for the list every two seconds and tells the gateway
what changed.

## What the driver does with it {#driver}

| The gateway asks for | The driver sends |
|---|---|
| a new sandbox | `status` (it must not exist), `reserve`, `provision`, `start`; `delete` if one fails, so that nothing of a failed launch is left |
| start of a stopped sandbox | `reserve` (the user it has), `provision` with new launch files (the token is kept if the gateway sends none), `start` |
| stop | `stop` |
| delete | `delete` |
| the sandboxes and their state | `list`, `status` |

At its own start the driver sends `describe`, to see that the helper
is there. The user comes before the launch files because they name it:
OpenShell's workload identity is part of what the gateway signs for a
launch.
