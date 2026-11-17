- A sandbox's supervisor runs behind a Landlock fence that names its own
  sandbox's files. Every supervisor is the same user, so file modes
  alone let one read another sandbox's credentials; where the SELinux
  module is not loaded, nothing else stood in the way. The unit starts
  the stock `openshell-supervisor` through a new program,
  `openshell-supervisor-fence`.
