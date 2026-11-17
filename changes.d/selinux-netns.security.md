- With the SELinux module, the unit that holds a sandbox's network
  namespace runs in a domain of its own, `openshell_sandbox_netns_t`,
  which may write network settings of that namespace and nothing else.
  Without the module, too, the unit keeps one capability of root's and
  the host is read-only for it.
- The driver's user unit gains no privileges and makes no namespaces
  (`NoNewPrivileges=`, `RestrictNamespaces=` and others).
