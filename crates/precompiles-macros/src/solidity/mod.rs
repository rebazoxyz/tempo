//! `#[solidity]` module attribute macro.
//!
//! This module provides a unified macro for defining Solidity-compatible types
//! in a single Rust module, eliminating the need for separate `#[interface]`,
//! `#[error]`, `#[event]`, and `#[derive(SolStruct)]` macros.
//!
//! # Advantages
//!
//! - **Correct selectors**: Struct types are fully resolved before selector computation
//! - **EIP-712 components**: Nested struct dependencies are properly tracked
//! - **Convention over configuration**: Names determine type semantics
//! - **Co-location**: All related types live in one module
//!
//! # Example
//!
//! ```ignore
//! #[solidity]
//! pub mod roles_auth {
//!     use super::*;
//!
//!     #[derive(Clone, Debug)]
//!     pub struct Transfer {
//!         pub from: Address,
//!         pub to: Address,
//!         pub amount: U256,
//!     }
//!
//!     pub enum PolicyType {
//!         Whitelist,
//!         Blacklist,
//!     }
//!
//!     pub enum Error {
//!         Unauthorized,
//!         InsufficientBalance { available: U256, required: U256 },
//!     }
//!
//!     pub enum Event {
//!         RoleMembershipUpdated {
//!             #[indexed] role: B256,
//!             #[indexed] account: Address,
//!             sender: Address,
//!             has_role: bool,
//!         },
//!     }
//!
//!     pub trait Interface {
//!         fn has_role(&self, account: Address, role: B256) -> Result<bool>;
//!         fn grant_role(&mut self, role: B256, account: Address) -> Result<()>;
//!     }
//! }
//! ```

mod common;
mod enum_gen;
mod interface_gen;
mod parser;
mod registry;
mod struct_gen;
#[cfg(test)]
mod test_utils;

use proc_macro2::TokenStream;
use quote::quote;
use syn::ItemMod;

use parser::parse_solidity_module;
use registry::TypeRegistry;

/// Main expansion entry point for `#[solidity]` attribute macro.
///
/// This function:
/// 1. Parses the module into IR
/// 2. Builds a type registry for ABI resolution
/// 3. Generates code for all types with full type knowledge
pub(crate) fn expand(item: ItemMod) -> syn::Result<TokenStream> {
    let module = parse_solidity_module(item)?;
    let registry = TypeRegistry::from_module(&module)?;

    let mod_name = &module.name;
    let vis = &module.vis;

    let imports: Vec<TokenStream> = module.imports.iter().map(|i| quote! { #i }).collect();

    let struct_impls: syn::Result<Vec<TokenStream>> = module
        .structs
        .iter()
        .map(|def| struct_gen::generate_struct(def, &registry))
        .collect();
    let struct_impls = struct_impls?;

    let unit_enum_impls: Vec<TokenStream> = module
        .unit_enums
        .iter()
        .map(enum_gen::generate_unit_enum)
        .collect();

    let error_impl = if let Some(ref def) = module.error {
        Some(enum_gen::generate_variant_enum(
            def,
            &registry,
            enum_gen::VariantEnumKind::Error,
        )?)
    } else {
        None
    };

    let event_impl = if let Some(ref def) = module.event {
        Some(enum_gen::generate_variant_enum(
            def,
            &registry,
            enum_gen::VariantEnumKind::Event,
        )?)
    } else {
        None
    };

    let interface_impl = if let Some(ref def) = module.interface {
        Some(interface_gen::generate_interface(def, &registry)?)
    } else {
        None
    };

    let other_items: Vec<TokenStream> = module.other_items.iter().map(|i| quote! { #i }).collect();

    Ok(quote! {
        #[allow(non_camel_case_types, non_snake_case, clippy::pub_underscore_fields, clippy::style, clippy::empty_structs_with_brackets)]
        #vis mod #mod_name {
            #(#imports)*

            #(#struct_impls)*

            #(#unit_enum_impls)*

            #error_impl

            #event_impl

            #interface_impl

            #(#other_items)*
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_integration() -> syn::Result<()> {
        let item: ItemMod = syn::parse2(quote! {
            pub mod test {
                use super::*;
                pub struct Transfer { pub from: Address, pub to: Address, pub amount: U256 }
                pub enum OrderStatus { Pending, Filled }
                pub enum Error { Unauthorized }
                pub enum Event { Transfer { #[indexed] from: Address, to: Address, amount: U256 } }
                pub trait Interface {
                    fn balance_of(&self, account: Address) -> Result<U256>;
                    fn transfer(&mut self, data: Transfer) -> Result<()>;
                }
            }
        })?;

        let module = parse_solidity_module(item)?;
        let registry = TypeRegistry::from_module(&module)?;

        assert_eq!(
            registry.resolve_abi(&syn::parse_quote!(Transfer))?,
            "(address,address,uint256)"
        );
        assert!(registry.is_unit_enum(&syn::parse_quote!(OrderStatus)));
        let sig = registry.compute_signature("transfer", &[syn::parse_quote!(Transfer)])?;
        assert_eq!(sig, "transfer((address,address,uint256))");

        Ok(())
    }

    #[test]
    fn test_expand_full_module() -> syn::Result<()> {
        let item: ItemMod = syn::parse2(quote! {
            pub mod example {
                use super::*;
                pub struct Transfer { pub from: Address, pub to: Address, pub amount: U256 }
                pub enum OrderStatus { Pending, Filled }
                pub enum Error { Unauthorized, InsufficientBalance { available: U256 } }
                pub enum Event { Transfer { #[indexed] from: Address, to: Address, amount: U256 } }
                pub trait Interface {
                    fn balance_of(&self, account: Address) -> Result<U256>;
                    fn transfer(&mut self, to: Address, amount: U256) -> Result<()>;
                }
            }
        })?;

        let code = expand(item)?.to_string();
        assert!(code.contains("mod example") && code.contains("struct Transfer"));
        assert!(
            code.contains("enum OrderStatus")
                && code.contains("enum Error")
                && code.contains("enum Event")
        );
        assert!(
            code.contains("trait Interface")
                && code.contains("balanceOfCall")
                && code.contains("transferCall")
        );
        Ok(())
    }

    #[test]
    fn test_expand_edge_cases() -> syn::Result<()> {
        // Empty module
        let item: ItemMod = syn::parse2(quote! { pub mod empty { use super::*; } })?;
        expand(item)?;

        // Nested structs only
        let item: ItemMod = syn::parse2(quote! {
            pub mod structs_only {
                pub struct Inner { pub value: U256 }
                pub struct Outer { pub inner: Inner, pub extra: Address }
            }
        })?;
        let code = expand(item)?.to_string();
        assert!(code.contains("struct Inner") && code.contains("struct Outer"));
        Ok(())
    }
}
