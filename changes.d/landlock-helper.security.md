- `openshell-systemd-helper` puts a Landlock fence around itself before
  it serves a request: its state directory, the sandbox units' drop-ins
  and its socket, and no TCP. It binds root, with or without the SELinux
  module. On a kernel that cannot do this (before Linux 6.2) the helper
  does not start; `--landlock best-effort` or `off` changes that.
