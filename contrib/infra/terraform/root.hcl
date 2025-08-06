generate "providers" {
    path = "provider.tf"
    if_exists = "overwrite_terragrunt"
    contents = <<EOF
provider "latitudesh" {
    auth_token = "${get_env("LATITUDESH_API_KEY")}"
}

terraform {
  required_providers {
    tailscale = {
      source  = "tailscale/tailscale"
      version = "0.21.1"
    }

    latitudesh = {
      source = "latitudesh/latitudesh"
      version = "2.3.0"
    }
  }
}

EOF
}