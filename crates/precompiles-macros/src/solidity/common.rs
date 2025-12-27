//! Shared utilities for code generation.

use alloy_sol_macro_expander::{
    SolInterfaceData, SolInterfaceKind, expand_sol_interface, expand_tokenize_simple, selector,
};
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};
use syn::Type;

use crate::utils::SolType;

use super::parser::EnumVariantDef;
use super::registry::TypeRegistry;

/// Generate param tuple from sol_types.
pub(super) fn make_param_tuple(sol_types: &[TokenStream]) -> TokenStream {
    if sol_types.is_empty() {
        quote! { () }
    } else {
        quote! { (#(#sol_types,)*) }
    }
}

/// Convert types to sol_data types.
pub(super) fn types_to_sol_types(types: &[syn::Type]) -> syn::Result<Vec<TokenStream>> {
    types
        .iter()
        .map(|ty| Ok(SolType::from_syn(ty)?.to_sol_data()))
        .collect()
}

/// Encoded parameter information for ABI generation.
pub(super) struct EncodedParams {
    pub param_tuple: TokenStream,
    pub tokenize_impl: TokenStream,
}

/// Encode parameters for ABI generation.
pub(super) fn encode_params(names: &[Ident], types: &[Type]) -> syn::Result<EncodedParams> {
    let sol_types = types_to_sol_types(types)?;
    let param_tuple = make_param_tuple(&sol_types);
    let tokenize_impl = expand_tokenize_simple(names, &sol_types);
    Ok(EncodedParams {
        param_tuple,
        tokenize_impl,
    })
}

/// Generate signature doc string with selector.
pub(super) fn signature_doc(kind: &str, signature: &str) -> String {
    let sel = selector(signature);
    format!(
        "{} with signature `{}` and selector `0x{}`.",
        kind,
        signature,
        hex::encode(sel)
    )
}

/// Generate a SolInterface container enum (Calls, Error, or Event).
///
/// Takes variant names, type names, signatures, and field counts to build
/// the `SolInterfaceData` and expand it.
///
/// NOTE: Generated container enums are always `pub` within the module,
/// regardless of the original item's visibility.
pub(super) fn generate_sol_interface_container(
    container_name: &str,
    variants: &[Ident],
    types: &[Ident],
    signatures: &[String],
    field_counts: &[usize],
    kind: SolInterfaceKind,
) -> TokenStream {
    let data = SolInterfaceData {
        name: format_ident!("{}", container_name),
        variants: variants.to_vec(),
        types: types.to_vec(),
        selectors: signatures.iter().map(selector).collect(),
        min_data_len: field_counts.iter().copied().min().unwrap_or(0) * 32,
        signatures: signatures.to_vec(),
        kind,
    };
    expand_sol_interface(data)
}

/// Generate Error container enum from variants.
pub(super) fn generate_error_container(
    variants: &[EnumVariantDef],
    registry: &TypeRegistry,
) -> syn::Result<TokenStream> {
    let names: Vec<Ident> = variants.iter().map(|v| v.name.clone()).collect();
    let signatures: syn::Result<Vec<String>> = variants
        .iter()
        .map(|v| registry.compute_signature_from_fields(&v.name.to_string(), &v.fields))
        .collect();
    let field_counts: Vec<usize> = variants.iter().map(|v| v.fields.len()).collect();
    Ok(generate_sol_interface_container(
        "Error",
        &names,
        &names,
        &signatures?,
        &field_counts,
        SolInterfaceKind::Error,
    ))
}

/// Generate Event container enum with IntoLogData impl and From conversions.
///
/// NOTE: Generated container enums are always `pub` within the module,
/// regardless of the original item's visibility.
pub(super) fn generate_event_container(variants: &[EnumVariantDef]) -> TokenStream {
    let names: Vec<&Ident> = variants.iter().map(|v| &v.name).collect();

    quote! {
        /// Container enum for all event types.
        #[derive(Clone, Debug, PartialEq, Eq)]
        pub enum Event {
            #(#[allow(missing_docs)] #names(#names),)*
        }

        #[automatically_derived]
        impl ::alloy::primitives::IntoLogData for Event {
            fn to_log_data(&self) -> ::alloy::primitives::LogData {
                match self { #(Self::#names(inner) => inner.to_log_data(),)* }
            }
            fn into_log_data(self) -> ::alloy::primitives::LogData {
                match self { #(Self::#names(inner) => inner.into_log_data(),)* }
            }
        }

        #(
            #[automatically_derived]
            impl ::core::convert::From<#names> for Event {
                #[inline]
                fn from(value: #names) -> Self {
                    Self::#names(value)
                }
            }
        )*
    }
}

/// Generate simple struct (unit or with named fields).
pub(super) fn generate_simple_struct(
    name: &Ident,
    fields: &[(&Ident, &Type)],
    doc: &str,
) -> TokenStream {
    if fields.is_empty() {
        quote! {
            #[doc = #doc]
            #[derive(Clone, Debug, PartialEq, Eq)]
            pub struct #name;
        }
    } else {
        let names: Vec<_> = fields.iter().map(|(n, _)| *n).collect();
        let types: Vec<_> = fields.iter().map(|(_, t)| *t).collect();
        quote! {
            #[doc = #doc]
            #[derive(Clone, Debug, PartialEq, Eq)]
            pub struct #name {
                #(pub #names: #types),*
            }
        }
    }
}
