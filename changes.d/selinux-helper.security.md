- With the SELinux module, `openshell-systemd-helper` runs in a domain of
  its own, `openshell_systemd_helper_t`, where it ran in the domain of
  any unconfined service: it reaches the sandboxes' files and the sandbox
  units, and no network.
