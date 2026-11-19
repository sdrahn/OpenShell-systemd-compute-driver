#!/usr/bin/python3
# SPDX-FileCopyrightText: Copyright (c) 2026 Daniel Rahn
# SPDX-License-Identifier: Apache-2.0
"""run-services.py _SERVICE DIR [URL REVISION]: run the OBS source
services of _SERVICE in DIR, as `osc service manualrun` runs them in a
package's checkout (_service and the spec): each in turn as /usr/lib/obs/service/<name> with its
parameters and --outdir, its files then moved into DIR, where the next
one finds them. With URL and REVISION, the tar_scm service that fetches
this repository takes them instead of its url and revision, so that CI
packs the commit under test (tar_scm takes no file:// URL: CI serves the
checkout with git daemon, git://). Needs the services installed (obs-service-
tar_scm, -recompress, -set_version, -cargo)."""

import os
import shutil
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET

THIS_REPO = "OpenShell-systemd-compute-driver"


def main(argv):
    if len(argv) not in (3, 5):
        sys.exit(__doc__)
    service_file, outdir = argv[1], os.path.abspath(argv[2])
    repo, revision = (argv[3], argv[4]) if len(argv) == 5 else (None, None)
    os.makedirs(outdir, exist_ok=True)
    # The package's checkout: _service and the spec, which set_version
    # gives the version of the driver's tarball.
    here = os.path.dirname(os.path.abspath(service_file))
    for f in os.listdir(here):
        if f == "_service" or f.endswith(".spec"):
            shutil.copy(os.path.join(here, f), outdir)
    for service in ET.parse(service_file).getroot().findall("service"):
        name = service.get("name")
        params = [(p.get("name"), (p.text or "").strip()) for p in service.findall("param")]
        if repo and name == "tar_scm" and any(k == "url" and THIS_REPO in v for k, v in params):
            params = [(k, repo if k == "url" else revision if k == "revision" else v) for k, v in params]
        with tempfile.TemporaryDirectory(dir=os.path.dirname(outdir)) as tmp:
            cmd = [f"/usr/lib/obs/service/{name}"]
            for k, v in params:
                cmd += [f"--{k}", v]
            cmd += ["--outdir", tmp]
            print("+", " ".join(cmd), flush=True)
            subprocess.run(cmd, cwd=outdir, check=True)
            for f in os.listdir(tmp):
                dst = os.path.join(outdir, f)
                if os.path.isdir(dst) and not os.path.islink(dst):
                    shutil.rmtree(dst)
                shutil.move(os.path.join(tmp, f), dst)
    # osc keeps what the services made; the uncompressed tarballs are
    # recompress's input and not sources of the spec.
    for f in sorted(os.listdir(outdir)):
        if f.endswith(".tar"):
            os.remove(os.path.join(outdir, f))
    for f in sorted(os.listdir(outdir)):
        print(f"{os.path.getsize(os.path.join(outdir, f)):>12}  {f}")


if __name__ == "__main__":
    main(sys.argv)
