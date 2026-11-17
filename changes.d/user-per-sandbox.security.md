- Every sandbox has a host user of its own. The helper gives it the
  first free user ID of a range (`--uid-range`,
  `OPENSHELL_SYSTEMD_UID_RANGE`, 70000 to 79999 unless set) and keeps it
  for the sandbox's life; its supervisor, its workload and its files
  are that user's. One sandbox's user is refused another's files and
  cannot signal another's processes by the kernel's ordinary checks,
  also on a host without the SELinux module and also signal 0. Before,
  every sandbox ran as `openshell-sandbox`, and only namespaces, SELinux
  categories and the supervisor's fence kept them apart.
