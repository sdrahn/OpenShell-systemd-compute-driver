- The driver's helper starts on a transactional system (SLES 16 or
  Leap 16 in immutable mode). The package ships the sandboxes' mount
  points `/.openshell/supervisor`, `/.openshell/channel` and `/sandbox`.
  Before, the helper made them each time it started, and on a read-only
  root it exited with `Read-only file system (os error 30)`. A mount
  point that is still missing is named in the helper's error.
