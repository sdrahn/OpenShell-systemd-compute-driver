- Creating a sandbox no longer fails now and then with "sandbox
  entered error phase while provisioning: ContainerStopped". While the
  driver launches a sandbox, its units are reported as starting; before,
  the driver's periodic check could report them as stopped between
  provisioning and start, and the gateway failed the creation.
