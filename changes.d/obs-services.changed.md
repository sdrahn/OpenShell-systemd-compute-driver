- The package's sources can be made in a build service project with
  `osc service manualrun`, as Custos's and mcp-gateway's: `_service`
  fetches the driver and OpenShell's source at the locked commit, and
  vendors the crates of each as `Cargo.lock` locks them. The crates
  are now two files, `vendor.tar.zst` for the driver and
  `vendor-openshell.tar.zst` for OpenShell, where `make-sources` and
  each release made one; the spec takes both.
