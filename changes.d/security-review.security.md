- The helper gives a supervisor only OpenShell's own environment
  variables (`OPENSHELL_…`, not `OPENSHELL_SYSTEMD_…`): a launch can no
  longer switch off the supervisor's Landlock fence or have a library
  loaded into it.
- The supervisor's unit holds its own sandbox's files and runtime
  directory and no other's, and the workload's unit nothing of the
  host's `/run` and shared memory of its own.
- With the SELinux module, a workload's domain cannot read its
  supervisor's credentials even where it could reach them.
- The helper sets owners and modes on the files it opened, not by their
  paths; keeps few of root's capabilities; runs its programs by their
  full paths; and does not start where SELinux is on but a label cannot
  be read.
- A sandbox keeps its SELinux categories when a launch fails or the
  helper starts without finding the policy module.
- An error of a start no longer carries the units' log lines.
