variable "tailscale_operator_version" {
  type = string
}

variable "cluster_name" {
  type        = string
  description = "Name of the Kubernetes cluster"
}

variable "argocd_version" {
  type        = string
  description = "ArgoCD Helm chart version"
}

variable "argocd_tailscale_hostname" {
  type        = string
  description = "Tailscale hostname for ArgoCD ingress"
}