- The package no longer builds and ships `openshell-supervisor` and
  `openshell-sandbox` in `/usr/libexec/openshell`: it requires the
  packages `openshell-supervisor` and `openshell-sandbox` of the
  OpenShell release it is built against, and the sandboxes run
  `/usr/bin/openshell-supervisor` and `/usr/bin/openshell-sandbox`,
  which the policy module labels. The repository that holds the driver
  has to hold those packages too.
