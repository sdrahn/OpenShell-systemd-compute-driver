- A guide for administrators and for those who work in a sandbox, in
  `docs/user-guide`: what the driver is and when to use it, installing
  and setting up, what a workload can and cannot do, network policy
  with a worked example, the security model, operation, troubleshooting
  with the error messages, a reference of every setting, the helper
  protocol, and which statements the tests hold. The README was the
  only documentation before and is now an overview.
- The package `openshell-driver-systemd-docs` installs the guide and
  the security review in `/usr/share/openshell-driver-systemd/docs`,
  with a collection file for mcp-docs, which then serves them to the
  agents on the host.
