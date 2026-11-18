- Creating or starting a sandbox no longer fails now and then with
  "sandbox entered error phase while provisioning: UnitsStarting". Units
  that are starting are reported with the reason `Starting`, which the
  gateway takes for a sandbox still coming up; it took `UnitsStarting`,
  the reason before, for an error. 0.1.3 reported a sandbox being
  launched with it and so did not fix the failure it was made for.
