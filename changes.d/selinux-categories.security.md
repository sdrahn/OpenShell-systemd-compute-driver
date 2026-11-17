- With the SELinux module, every sandbox has a pair of categories of its
  own. Its supervisor, its workload and its files carry them, so the
  policy refuses one sandbox what is another's, where only the
  namespaces kept them apart before. A sandbox that exists gets its
  pair when it is next started through the gateway.
