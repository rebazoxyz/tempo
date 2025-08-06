include "kubernetes" {
  path = find_in_parent_folders("kubernetes.hcl")
}

include "root" {
  path = find_in_parent_folders("root.hcl")
}

terraform {
  source = find_in_parent_folders("modules/kubernetes/bootstrap")
}


inputs = {
  # Cluster configuration
  cluster_name = "dev-dal-01"
  
  # Tailscale operator configuration
  tailscale_operator_version = "1.76.6"
  
  # ArgoCD configuration
  argocd_version            = "7.6.12"
  argocd_tailscale_hostname = "argocd-dev-dal-01"
}