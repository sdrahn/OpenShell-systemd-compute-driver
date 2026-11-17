- `openshell-driver-systemd --bind-address`: the driver no longer
  listens on TCP, where it served anyone on the host. `--bind-socket`
  is required.
- `OPENSHELL_SYSTEMD_SUPERVISOR_LANDLOCK`: the supervisor's fence is
  set with `--landlock` in the unit only.
