//! Utility functions for the contract macro implementation.

use alloy::primitives::U256;
use syn::{Attribute, Lit, Type};

/// Converts a string from CamelCase or snake_case to snake_case.
/// Preserves SCREAMING_SNAKE_CASE, as those are assumed to be constant/immutable names.
pub(crate) fn normalize_to_snake_case(s: &str) -> String {
    let constant = s.to_uppercase();
    if s == constant {
        return constant;
    }

    let mut result = String::with_capacity(s.len() + 4);
    let mut chars = s.chars().peekable();
    let mut prev_upper = false;

    while let Some(c) = chars.next() {
        if c.is_uppercase() {
            if !result.is_empty()
                && (!prev_upper || chars.peek().is_some_and(|&next| next.is_lowercase()))
            {
                result.push('_');
            }
            result.push(c.to_ascii_lowercase());
            prev_upper = true;
        } else {
            result.push(c);
            prev_upper = false;
        }
    }

    result
}

/// Extracts `#[slot(N)]` and `#[map = "..."]` attributes from a field's attributes.
///
/// This function iterates through the attributes a single time to find both
/// the slot and map values. It returns a tuple containing the slot number
/// (if present) and the map string value (if present). The map value is
/// normalized to snake_case.
pub(crate) fn extract_attributes(
    attrs: &[Attribute],
) -> syn::Result<(Option<U256>, Option<String>)> {
    let mut slot_attr: Option<U256> = None;
    let mut map_attr: Option<String> = None;

    for attr in attrs {
        // Extract `#[slot(N)]` attribute
        if attr.path().is_ident("slot") {
            if slot_attr.is_some() {
                return Err(syn::Error::new_spanned(attr, "Duplicate 'slot' attribute"));
            }

            let value: Lit = attr.parse_args()?;
            if let Lit::Int(lit_int) = value {
                let lit_str = lit_int.to_string();
                let slot = if let Some(hex) = lit_str.strip_prefix("0x") {
                    U256::from_str_radix(hex, 16)
                } else {
                    U256::from_str_radix(&lit_str, 10)
                }
                .map_err(|_| syn::Error::new_spanned(&lit_int, "Invalid slot number"))?;
                slot_attr = Some(slot);
            } else {
                return Err(syn::Error::new_spanned(
                    value,
                    "slot attribute must be an integer literal",
                ));
            }
        }
        // Extract `#[map = "..."]` attribute
        else if attr.path().is_ident("map") {
            if map_attr.is_some() {
                return Err(syn::Error::new_spanned(attr, "Duplicate 'map' attribute"));
            }

            let meta: syn::Meta = attr.meta.clone();
            if let syn::Meta::NameValue(meta_name_value) = meta {
                if let syn::Expr::Lit(expr_lit) = meta_name_value.value {
                    if let Lit::Str(lit_str) = expr_lit.lit {
                        map_attr = Some(normalize_to_snake_case(&lit_str.value()));
                    } else {
                        return Err(syn::Error::new_spanned(
                            expr_lit,
                            "map attribute must be a string literal",
                        ));
                    }
                }
            } else {
                return Err(syn::Error::new_spanned(
                    attr,
                    "map attribute must use the form: #[map = \"value\"]",
                ));
            }
        }
    }

    Ok((slot_attr, map_attr))
}

/// Extracts the type parameters from Mapping<K, V>.
///
/// Returns Some((key_type, value_type)) if the type is a Mapping, None otherwise.
pub(crate) fn extract_mapping_types(ty: &Type) -> Option<(&Type, &Type)> {
    if let Type::Path(type_path) = ty {
        let last_segment = type_path.path.segments.last()?;

        // Check if the type is named "Mapping"
        if last_segment.ident != "Mapping" {
            return None;
        }

        // Extract generic arguments
        if let syn::PathArguments::AngleBracketed(args) = &last_segment.arguments {
            let mut iter = args.args.iter();

            // First argument: key type
            let key_type = if let Some(syn::GenericArgument::Type(ty)) = iter.next() {
                ty
            } else {
                return None;
            };

            // Second argument: value type
            let value_type = if let Some(syn::GenericArgument::Type(ty)) = iter.next() {
                ty
            } else {
                return None;
            };

            return Some((key_type, value_type));
        }
    }
    None
}

/// Checks if a type is the unit type `()`.
pub(crate) fn is_unit(ty: &Type) -> bool {
    matches!(ty, Type::Tuple(tuple) if tuple.elems.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn test_normalize_to_snake_case() {
        assert_eq!(normalize_to_snake_case("balanceOf"), "balance_of");
        assert_eq!(normalize_to_snake_case("transferFrom"), "transfer_from");
        assert_eq!(normalize_to_snake_case("name"), "name");
        assert_eq!(normalize_to_snake_case("already_snake"), "already_snake");
        assert_eq!(
            normalize_to_snake_case("updateQuoteToken"),
            "update_quote_token"
        );
        assert_eq!(
            normalize_to_snake_case("DOMAIN_SEPARATOR"),
            "DOMAIN_SEPARATOR"
        );
        assert_eq!(normalize_to_snake_case("ERC20Token"), "erc20_token");
    }

    #[test]
    fn test_extract_mapping_types() {
        // Test simple mapping
        let ty: Type = parse_quote!(Mapping<Address, U256>);
        let result = extract_mapping_types(&ty);
        assert!(result.is_some());

        // Test nested mapping
        let ty: Type = parse_quote!(Mapping<Address, Mapping<Address, U256>>);
        let result = extract_mapping_types(&ty);
        assert!(result.is_some());

        // Test non-mapping type
        let ty: Type = parse_quote!(String);
        let result = extract_mapping_types(&ty);
        assert!(result.is_none());

        // Test non-mapping generic type
        let ty: Type = parse_quote!(Vec<u8>);
        let result = extract_mapping_types(&ty);
        assert!(result.is_none());
    }

    #[test]
    fn test_is_unit_type() {
        let unit: Type = parse_quote!(());
        assert!(is_unit(&unit));

        let non_unit: Type = parse_quote!(bool);
        assert!(!is_unit(&non_unit));
    }
}
