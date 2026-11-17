- A sandbox's own variables (`--env`, `--env-from`, and a template's)
  reach its workload and the commands run in it. Before, the workload
  had only a `PATH`: `openshell-sandbox` takes them from the launch's
  boundary configuration, where the driver did not put them.
