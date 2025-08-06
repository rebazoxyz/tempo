resource "kubernetes_namespace" "tailscale" {
  metadata {
    name = "tailscale"
  }
}

resource "tailscale_oauth_client" "kubernetes_operator" {
  description = "OAuth client for Kubernetes operator on ${var.cluster_name}"
  
  tags = [
    "tag:k8s-operator"
  ]

  scopes = ["devices:core", "auth_keys"]
}

resource "helm_release" "tailscale_operator" {
  name      = "tailscale-operator"
  chart     = "tailscale-operator"
  repository = "https://pkgs.tailscale.com/helmcharts"
  version   = var.tailscale_operator_version
  namespace = kubernetes_namespace.tailscale.metadata[0].name

  set = [
    {
      name  = "oauth.clientId"
      value = tailscale_oauth_client.kubernetes_operator.id
    },
    {
      name  = "oauth.clientSecret"
      value = tailscale_oauth_client.kubernetes_operator.key
    }
  ]

  depends_on = [
    kubernetes_namespace.tailscale,
    tailscale_oauth_client.kubernetes_operator
  ]
}

resource "kubernetes_namespace" "argocd" {
  metadata {
    name = "argocd"
  }
}

resource "helm_release" "argocd" {
  name       = "argocd"
  repository = "https://argoproj.github.io/argo-helm"
  chart      = "argo-cd"
  version    = var.argocd_version
  namespace  = kubernetes_namespace.argocd.metadata[0].name

  set = [
    {
      name  = "server.service.type"
      value = "ClusterIP"
    },
    {
      name  = "server.ingress.enabled"
      value = "true"
    },
    {
      name  = "server.ingress.ingressClassName"
      value = "tailscale"
    },
    {
      name  = "server.ingress.annotations.tailscale\\.com/expose"
      value = "true"
    },
    {
      name  = "server.ingress.annotations.tailscale\\.com/hostname"
      value = var.argocd_tailscale_hostname
    },
    {
      name  = "server.ingress.hosts[0].host"
      value = var.argocd_tailscale_hostname
    },
    {
      name  = "server.ingress.hosts[0].paths[0].path"
      value = "/"
    },
    {
      name  = "server.ingress.hosts[0].paths[0].pathType"
      value = "Prefix"
    },
    {
      name  = "server.ingress.tls[0].hosts[0]"
      value = var.argocd_tailscale_hostname
    },
    {
      name  = "configs.params.server\\.insecure"
      value = "true"
    }
  ]

  depends_on = [
    kubernetes_namespace.argocd,
    helm_release.tailscale_operator
  ]
}