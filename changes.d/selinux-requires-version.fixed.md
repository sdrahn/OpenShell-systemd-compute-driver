- `openshell-driver-systemd-selinux` requires `selinux-policy` by its
  version only. Before, it required the release of the build's
  `selinux-policy` too, so a build for SLES 16.1 (Package Hub) could
  not be installed on a host with an older release of the same policy
  version ("nothing provides 'selinux-policy >= …'").
