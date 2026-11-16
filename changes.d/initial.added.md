- `openshell-driver-systemd`, an external compute driver for the
  OpenShell gateway that runs each sandbox as systemd units on the
  gateway's host, with its privileged helper
  `openshell-systemd-helper` and the unit templates.
- An SELinux policy module with a domain for a sandbox's workload and
  one for its supervisor.
- A package for SLES 16 and openSUSE Leap 16 that also holds
  `openshell-supervisor` and `openshell-sandbox` of the OpenShell
  release the driver is built against.
