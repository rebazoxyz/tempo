//! Unified Calls/Error/Event enum generation for `#[contract(solidity(...))]`.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Ident, Path};

use crate::{solidity::AbiType, utils::to_pascal_case};

pub(crate) fn generate_composition(
    struct_name: &Ident,
    modules: &[Path],
) -> syn::Result<TokenStream> {
    if modules.is_empty() {
        return Ok(TokenStream::new());
    }

    let helpers = generate_selector_helpers();
    let calls = generate_composed_sol_interface(struct_name, modules, AbiType::Calls);
    let error = generate_composed_sol_interface(struct_name, modules, AbiType::Error);
    let event = generate_event_enum(struct_name, modules);

    Ok(quote! { #helpers #calls #error #event })
}

fn variant_name(path: &Path) -> Ident {
    let seg = path.segments.last().expect("non-empty path");
    format_ident!("{}", to_pascal_case(&seg.ident.to_string()))
}

/// Generates a private helper module with const fn's for concatenating selector arrays.
fn generate_selector_helpers() -> TokenStream {
    quote! {
        #[doc(hidden)]
        mod __compose_helpers {
            pub const fn concat_4<const N: usize, const M: usize>(
                a: [&'static [[u8; 4]]; N]
            ) -> [[u8; 4]; M] {
                let mut r = [[0u8; 4]; M];
                let (mut i, mut n_idx) = (0, 0);
                while n_idx < N {
                    let s = a[n_idx];
                    let mut j = 0;
                    while j < s.len() { r[i] = s[j]; i += 1; j += 1; }
                    n_idx += 1;
                }
                r
            }

            pub const fn concat_b256<const N: usize, const M: usize>(
                a: [&'static [::alloy::primitives::B256]; N]
            ) -> [::alloy::primitives::B256; M] {
                let mut r = [::alloy::primitives::B256::ZERO; M];
                let (mut i, mut n_idx) = (0, 0);
                while n_idx < N {
                    let s = a[n_idx];
                    let mut j = 0;
                    while j < s.len() { r[i] = s[j]; i += 1; j += 1; }
                    n_idx += 1;
                }
                r
            }
        }
    }
}

/// Unified generator for Calls and Error composition enums.
///
/// Note: `AbiType::Event` is handled separately by `generate_event_enum`.
fn generate_composed_sol_interface(
    struct_name: &Ident,
    modules: &[Path],
    kind: AbiType,
) -> TokenStream {
    let (type_suffix, error_msg, generate_from_impls) = match kind {
        AbiType::Calls => ("Calls", "calldata too short", false),
        AbiType::Error => ("Error", "error data too short", true),
        AbiType::Event => unreachable!("use generate_event_enum for events"),
    };

    let name = format_ident!("{}{}", struct_name, type_suffix);
    let inner_type = format_ident!("{}", type_suffix);
    let variants: Vec<_> = modules.iter().map(variant_name).collect();
    let n = modules.len();

    let decls: Vec<_> = variants
        .iter()
        .zip(modules)
        .map(|(v, m)| {
            quote! { #v(#m::#inner_type) }
        })
        .collect();

    let selectors: Vec<_> = modules
        .iter()
        .map(|m| {
            quote! { #m::#inner_type::SELECTORS }
        })
        .collect();

    let counts: Vec<_> = modules
        .iter()
        .map(|m| {
            quote! { <#m::#inner_type as ::alloy_sol_types::SolInterface>::COUNT }
        })
        .collect();

    let decode: Vec<_> = variants
        .iter()
        .zip(modules)
        .map(|(v, m)| {
            quote! {
                if <#m::#inner_type as ::alloy_sol_types::SolInterface>::valid_selector(sel) {
                    return <#m::#inner_type as ::alloy_sol_types::SolInterface>::abi_decode(data).map(Self::#v);
                }
            }
        })
        .collect();

    let sel_match: Vec<_> = variants
        .iter()
        .zip(modules)
        .map(|(v, m)| {
            quote! { Self::#v(inner) => <#m::#inner_type as ::alloy_sol_types::SolInterface>::selector(inner) }
        })
        .collect();

    let size_match: Vec<_> = variants
        .iter()
        .zip(modules)
        .map(|(v, m)| {
            quote! { Self::#v(inner) => <#m::#inner_type as ::alloy_sol_types::SolInterface>::abi_encoded_size(inner) }
        })
        .collect();

    let enc_match: Vec<_> = variants
        .iter()
        .zip(modules)
        .map(|(v, m)| {
            quote! { Self::#v(inner) => <#m::#inner_type as ::alloy_sol_types::SolInterface>::abi_encode_raw(inner, out) }
        })
        .collect();

    let from_impls = if generate_from_impls {
        let impls: Vec<_> = variants
            .iter()
            .zip(modules)
            .map(|(v, m)| {
                quote! { impl From<#m::#inner_type> for #name { #[inline] fn from(e: #m::#inner_type) -> Self { Self::#v(e) } } }
            })
            .collect();
        quote! { #(#impls)* }
    } else {
        TokenStream::new()
    };

    let inherent_selector_method = if matches!(kind, AbiType::Error) {
        quote! { #[inline] pub fn selector(&self) -> [u8;4] { match self { #(#sel_match),* } } }
    } else {
        TokenStream::new()
    };

    let trait_selector = match kind {
        AbiType::Calls => quote! { match self { #(#sel_match),* } },
        AbiType::Error => quote! { #name::selector(self) },
        AbiType::Event => unreachable!(),
    };

    quote! {
        #[derive(Clone, Debug, PartialEq, Eq)]
        #[allow(non_camel_case_types, clippy::large_enum_variant)]
        pub enum #name { #(#decls),* }

        impl #name {
            pub const SELECTORS: &'static [[u8; 4]] = &{
                const TOTAL: usize = #(#selectors.len())+*;
                __compose_helpers::concat_4::<#n, TOTAL>([#(#selectors),*])
            };

            #[inline] pub fn valid_selector(s: [u8;4]) -> bool { Self::SELECTORS.contains(&s) }

            #inherent_selector_method

            pub fn abi_decode(data: &[u8]) -> ::alloy_sol_types::Result<Self> {
                let sel: [u8;4] = data.get(..4).and_then(|s| s.try_into().ok())
                    .ok_or_else(|| ::alloy_sol_types::Error::Other(#error_msg.into()))?;
                #(#decode)*
                Err(::alloy_sol_types::Error::unknown_selector(<Self as ::alloy_sol_types::SolInterface>::NAME, sel))
            }
        }

        impl ::alloy_sol_types::SolInterface for #name {
            const NAME: &'static str = stringify!(#name);
            const MIN_DATA_LENGTH: usize = 0;
            const COUNT: usize = #(#counts)+*;
            #[inline] fn selector(&self) -> [u8;4] { #trait_selector }
            #[inline] fn selector_at(i: usize) -> Option<[u8;4]> { Self::SELECTORS.get(i).copied() }
            #[inline] fn valid_selector(s: [u8;4]) -> bool { Self::valid_selector(s) }
            #[inline] fn abi_decode_raw(sel: [u8;4], data: &[u8]) -> ::alloy_sol_types::Result<Self> {
                let mut buf = Vec::with_capacity(4 + data.len()); buf.extend_from_slice(&sel); buf.extend_from_slice(data);
                Self::abi_decode(&buf)
            }
            #[inline] fn abi_decode_raw_validate(sel: [u8;4], data: &[u8]) -> ::alloy_sol_types::Result<Self> { Self::abi_decode_raw(sel, data) }
            #[inline] fn abi_encoded_size(&self) -> usize { match self { #(#size_match),* } }
            #[inline] fn abi_encode_raw(&self, out: &mut Vec<u8>) { match self { #(#enc_match),* } }
        }

        #from_impls
    }
}

fn generate_event_enum(struct_name: &Ident, modules: &[Path]) -> TokenStream {
    let name = format_ident!("{}Event", struct_name);
    let variants: Vec<_> = modules.iter().map(variant_name).collect();
    let n = modules.len();

    let decls: Vec<_> = variants
        .iter()
        .zip(modules)
        .map(|(v, m)| quote! { #v(#m::Event) })
        .collect();

    let selectors: Vec<_> = modules
        .iter()
        .map(|m| quote! { #m::Event::SELECTORS })
        .collect();

    let to_log: Vec<_> = variants
        .iter()
        .zip(modules)
        .map(|(v, m)| {
            quote! { Self::#v(e) => <#m::Event as ::alloy::primitives::IntoLogData>::to_log_data(e) }
        })
        .collect();

    let into_log: Vec<_> = variants
        .iter()
        .zip(modules)
        .map(|(v, m)| {
            quote! { Self::#v(e) => <#m::Event as ::alloy::primitives::IntoLogData>::into_log_data(e) }
        })
        .collect();

    let from_impls: Vec<_> = variants
        .iter()
        .zip(modules)
        .map(|(v, m)| {
            quote! { impl From<#m::Event> for #name { #[inline] fn from(e: #m::Event) -> Self { Self::#v(e) } } }
        })
        .collect();

    quote! {
        #[derive(Clone, Debug, PartialEq, Eq)]
        #[allow(non_camel_case_types, clippy::large_enum_variant)]
        pub enum #name { #(#decls),* }

        impl #name {
            pub const SELECTORS: &'static [::alloy::primitives::B256] = &{
                const TOTAL: usize = #(#selectors.len())+*;
                __compose_helpers::concat_b256::<#n, TOTAL>([#(#selectors),*])
            };
        }

        #[automatically_derived]
        impl ::alloy::primitives::IntoLogData for #name {
            fn to_log_data(&self) -> ::alloy::primitives::LogData { match self { #(#to_log),* } }
            fn into_log_data(self) -> ::alloy::primitives::LogData { match self { #(#into_log),* } }
        }

        #(#from_impls)*
    }
}
