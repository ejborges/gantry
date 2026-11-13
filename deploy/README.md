# Deployment examples

Starting points for installations larger than one server. The supported installation is the
RPM packages on Enterprise Linux (see `packaging/` and the manual's *System administration*
guide); these files show how the same pieces are arranged across several machines or under an
orchestrator.

| Directory | What it is |
| --- | --- |
| `multi-node/` | Configuration for application nodes, worker nodes and an nginx load balancer in front of them. |
| `container/` | A Containerfile that builds an image from the RPMs on a UBI base, for Podman or an orchestrator. |
| `kubernetes/` | Example manifests: application and worker Deployments, Service, Ingress, configuration. |

These are examples to adapt: replace host names, addresses and secrets. They were written
alongside this release but were not executed as part of its test suite, unlike the packages'
contents. Read the manual's *Scaling out* page first; most installations never need them.
