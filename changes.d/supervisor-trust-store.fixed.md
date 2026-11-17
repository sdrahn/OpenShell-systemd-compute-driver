- A sandbox's supervisor reads the host's CA certificates. Its
  Landlock fence let it read `/etc`, but on SLES and openSUSE
  `/etc/ssl/ca-bundle.pem` is a link into `/var/lib/ca-certificates`,
  so it fell back to the CA certificates built into OpenShell, and a
  host with a private CA, added to the host's trust store, could not be
  reached through it ("unknown certificate authority").
