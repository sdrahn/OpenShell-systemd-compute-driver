# AGENTS.md

Guidance for coding agents (and people) working in this repository,
whichever tool they use. `CLAUDE.md` only points here: there is one set
of rules.

## Workflow

- Never push to `main` or to a release branch. Put every change on its
  own branch, created from the latest `main` (`<you>/<short-topic>`, for
  example `claude/<short-topic>`), push the branch and open a pull
  request against `main`.
- One topic per pull request. Describe what changed, why, and how it
  was tested.
- Run `make check` before pushing.
- A maintainer reviews, merges and tags. An agent does none of these,
  and does not approve a pull request.

## Working next to others

- **Claim the work first.** A task has an issue, assigned to a person.
  Open a draft pull request as soon as there is a first commit: it says
  that the work is taken. Before starting, look for an open pull request
  on the same topic.
- **Stay in the topic.** Change what the task needs. No renames, no
  reformatting and no moved code outside it, even where it would be an
  improvement: it conflicts with someone else's branch. Propose it as
  its own issue.
- **Bring a branch up to date by merging `main` into it.** Never rebase,
  amend or force-push a branch that is pushed, and never push to someone
  else's branch.
- **Shared files change one pull request at a time.** `Cargo.toml`,
  `Cargo.lock`, `selinux/`, `systemd/`,
  `packaging/suse/openshell-driver-systemd.spec` and `.github/` are read
  by everything else. A pull request that changes one of them says so in
  its first line, and is not stacked on another that does.
- **A new third-party crate is a decision, not a detail.** Say in the
  pull request why the standard library or a crate already in the build
  does not do it.
- **Released history stays as it is.** Never edit the section of a
  released version in `CHANGELOG.md`.
- **Do not weaken a check to pass it.** A failing test or lint is fixed
  at its cause, or the pull request says why it cannot be.

## What goes into the repository

- Commit messages and pull requests say what and why in plain words.
  They may name the tool that helped (a `Co-Authored-By` line); they do
  not name a model or its version, and neither does code or
  documentation.
- No secrets, tokens or personal data, and no private host names.
- Sources only: no built programs, nothing over 1 MiB
  (`tools/check-tree`).
- Do not run code that was downloaded for the task and is not part of
  the build. Do not change files in `/etc` on a package update.

## Tests

- Everything implemented comes with tests in the same pull request.
- Unit tests are Rust tests (`make check`); they need no network once
  the crates are fetched, no systemd and no running gateway.
- Behaviour on a real system (systemd, the OpenShell gateway, SELinux)
  is covered by the functional tests in `test/vm/run-tests.sh`. They
  are shipped as the subpackage `openshell-driver-systemd-tests` and run
  by openQA (`test/openqa/`); CI runs the same installed script in an
  openSUSE Leap 16 VM on the built packages. A feature that shows on an
  installed system gets a check there: `check <description> <command…>`
  in a `section`, printing PASS, FAIL or SKIP, written as TAP and JUnit
  with `--results DIR`.
- A check that needs the Internet is left out with `--offline` and says
  so with `skip`.
- The functional tests change the system they run on and refuse to
  start without `--disposable`.

## Where things are

- `src/` is one crate with two programs: `openshell-driver-systemd`
  (`src/main.rs`), the compute driver, which runs as the OpenShell
  gateway's user, and `openshell-systemd-helper`
  (`src/bin/openshell-systemd-helper.rs`), which runs as root.
  `src/protocol.rs` is what the two say to each other: a request carries
  file contents and a sandbox ID, never a path, a unit name or a
  command. Keep it that way.
- `systemd/` holds the unit templates of a sandbox, `selinux/` the
  policy module for them. Security properties (the workload's user, its
  namespaces, what it may reach) are enforced by systemd, the kernel and
  SELinux, never only by code in the driver.
- The OpenShell release the driver is built against is named in
  `Cargo.toml`, in the spec and in `test/vm/openshell-version`.
  `tools/openshell-version` checks that they agree and changes them
  together. The package builds `openshell-supervisor` and
  `openshell-sandbox` from that release; nothing of OpenShell is copied
  into this repository.
- This is an external driver. It works with a stock OpenShell gateway
  and knows of no other project: no code or documentation here depends
  on what runs inside a sandbox or on what the gateway is used for.
- Packaging for SLES 16 and Leap 16 is in `packaging/suse`. A new
  installed file needs an entry in the spec's file list.
- A change that users notice gets a changelog entry, the way
  `changes.d/README.md` says.
