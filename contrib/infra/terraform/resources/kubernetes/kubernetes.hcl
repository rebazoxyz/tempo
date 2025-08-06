locals {
  kubernetes_vars = read_terragrunt_config(find_in_parent_folders("cluster.hcl"))
}

generate "kubernetes.tf" {
  path      = "kubernetes.tf"
  if_exists = "overwrite_terragrunt"

  contents  = <<EOF
  provider "kubernetes" {
    host                   = "${local.kubernetes_vars.locals.kube_host}"
    token                  = "${local.kubernetes_vars.locals.kube_token}"
  }

  provider "helm" {
    kubernetes = {
      host                   = "${local.kubernetes_vars.locals.kube_host}"
      token                  = "${local.kubernetes_vars.locals.kube_token}"
    }
  }

  provider "tailscale" {}
EOF
}
