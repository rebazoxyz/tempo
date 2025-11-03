//! Interface parsing and function extraction for contract macro.
//!
//! This module handles parsing the `#[contract(Interface1, Interface2)]` attribute and
//! extracting interface function signatures for trait generation.

use crate::utils::{self, try_extract_type_ident};
use proc_macro2::TokenStream;
use quote::quote;
use syn::{Ident, Type};

// TODO(rusowsky): Implement automatic method discovery from sol! generated interfaces.
mod registry;
use registry::*;

/// Stores both Rust-style (snake_case) and Solidity-style (camelCase) parameter names.
///
/// This allows generated trait methods to use idiomatic Rust naming while still correctly
/// accessing fields on Alloy-generated `solCall` structs which preserve Solidity naming.
#[derive(Debug, Clone)]
pub(crate) struct ParamName {
    /// Rust-style parameter name (snake_case) for trait method signatures
    pub rust: String,
    /// Solidity-style parameter name (camelCase) for accessing `solCall` struct fields
    pub sol: &'static str,
}

impl ParamName {
    pub(crate) fn new(solidity: &'static str) -> Self {
        Self {
            rust: utils::normalize_to_snake_case(solidity),
            sol: solidity,
        }
    }
}

/// Represents a single function from a sol! interface.
#[derive(Debug, Clone)]
pub(crate) struct InterfaceFunction {
    /// Function name, normalized to snake_case
    pub name: &'static str,
    /// Function parameters as (ParamName, type) pairs
    pub params: Vec<(ParamName, Type)>,
    /// Return type of the function
    pub return_type: Type,
    /// Whether this is a view function
    pub is_view: bool,
    /// Path to the Call struct for this function
    pub call_type_path: TokenStream,
}

/// Represents a single event from a sol! interface.
#[derive(Debug, Clone)]
pub(crate) struct InterfaceEvent {
    /// Event name, normalized to snake_case
    pub name: &'static str,
    /// Event parameters as (name, type, indexed) tuples
    pub params: Vec<(&'static str, Type, bool)>,
    /// Path to the Event struct for this event
    pub event_type_path: TokenStream,
}

/// Represents a single error from a sol! interface.
#[derive(Debug, Clone)]
pub(crate) struct InterfaceError {
    /// Error name
    pub name: &'static str,
    /// Error parameters as (name, type) pairs
    pub params: Vec<(&'static str, Type)>,
    /// Path to the Error struct for this error
    pub error_type_path: TokenStream,
}

/// Complete interface metadata including functions, events, and errors.
#[derive(Debug, Clone)]
pub(crate) struct Interface {
    /// Function definitions
    pub functions: Vec<InterfaceFunction>,
    /// Event definitions
    pub events: Vec<InterfaceEvent>,
    /// Error definitions
    pub errors: Vec<InterfaceError>,
}

/// Classification of function types for dispatcher routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FunctionKind {
    /// View function with no parameters
    Metadata,
    /// View function with parameters
    View,
    /// Mutating function returning
    Mutate,
    /// Mutating function returning
    MutateVoid,
}

impl InterfaceFunction {
    pub(crate) fn kind(&self) -> FunctionKind {
        match self.is_view {
            true if self.params.is_empty() => FunctionKind::Metadata,
            true => FunctionKind::View,
            false if utils::is_unit(&self.return_type) => FunctionKind::MutateVoid,
            false => FunctionKind::Mutate,
        }
    }
}

/// Constructs the event enum type path from an interface identifier.
/// Convention: ITIP20 -> ITIP20Events
pub(crate) fn get_event_enum_path(interface_ident: &Ident) -> syn::Result<TokenStream> {
    let event_enum_ident = format!("{interface_ident}Events");
    let event_enum: Ident = syn::parse_str(&event_enum_ident).expect("Valid identifier");
    Ok(quote!(#event_enum))
}

// TODO(rusowsky): Implement automatic method discovery from sol! generated interfaces.
pub(crate) fn parse_interface(interface_ident: &Ident) -> syn::Result<Interface> {
    get_interface_metadata(interface_ident)
}

// TODO(rusowsky): Implement automatic method discovery from sol! generated interfaces.
fn get_interface_metadata(interface_ident: &Ident) -> syn::Result<Interface> {
    let interface_name = interface_ident.to_string();
    match interface_name.as_str() {
        // crates/contracts/src/precompiles/tip20.rs
        "ITIP20" => Ok(Interface {
            functions: tip20::get_functions(interface_ident),
            events: tip20::get_events(interface_ident),
            errors: tip20::get_errors(interface_ident),
        }),
        "ITIP20Rewards" => Ok(Interface {
            functions: tip20_rewards::get_functions(interface_ident),
            events: tip20_rewards::get_events(interface_ident),
            errors: tip20_rewards::get_errors(interface_ident),
        }),
        "IRolesAuth" => Ok(Interface {
            functions: roles_auth::get_functions(interface_ident),
            events: roles_auth::get_events(interface_ident),
            errors: roles_auth::get_errors(interface_ident),
        }),

        // crates/contracts/src/precompiles/tip20_factory.rs
        "ITIP20Factory" => Ok(Interface {
            functions: tip20_factory::get_functions(interface_ident),
            events: tip20_factory::get_events(interface_ident),
            errors: tip20_factory::get_errors(interface_ident),
        }),

        // crates/contracts/src/precompiles/tip20_rewards_registry.rs
        "ITIP20RewardsRegistry" => Ok(Interface {
            functions: tip20_rewards_registry::get_functions(interface_ident),
            events: tip20_rewards_registry::get_events(interface_ident),
            errors: tip20_rewards_registry::get_errors(interface_ident),
        }),

        // crates/contracts/src/precompiles/tip403_registry.rs
        "ITIP403Registry" => Ok(Interface {
            functions: tip403_registry::get_functions(interface_ident),
            events: tip403_registry::get_events(interface_ident),
            errors: tip403_registry::get_errors(interface_ident),
        }),

        // crates/contracts/src/precompiles/tip4217_registry.rs
        "ITIP4217Registry" => Ok(Interface {
            functions: tip4217_registry::get_functions(interface_ident),
            events: tip4217_registry::get_events(interface_ident),
            errors: tip4217_registry::get_errors(interface_ident),
        }),

        // crates/contracts/src/precompiles/tip_fee_manager.rs
        "IFeeManager" => Ok(Interface {
            functions: fee_manager::get_functions(interface_ident),
            events: fee_manager::get_events(interface_ident),
            errors: fee_manager::get_errors(interface_ident),
        }),
        "ITIPFeeAMM" => Ok(Interface {
            functions: tip_fee_amm::get_functions(interface_ident),
            events: tip_fee_amm::get_events(interface_ident),
            errors: tip_fee_amm::get_errors(interface_ident),
        }),

        // crates/contracts/src/precompiles/stablecoin_exchange.rs
        "IStablecoinExchange" => Ok(Interface {
            functions: stablecoin_exchange::get_functions(interface_ident),
            events: stablecoin_exchange::get_events(interface_ident),
            errors: stablecoin_exchange::get_errors(interface_ident),
        }),

        // crates/contracts/src/precompiles/nonce.rs
        "INonce" => Ok(Interface {
            functions: nonce::get_functions(interface_ident),
            events: nonce::get_events(interface_ident),
            errors: nonce::get_errors(interface_ident),
        }),

        // crates/contracts/src/precompiles/tip_account_registrar.rs
        "ITipAccountRegistrar" => Ok(Interface {
            functions: tip_account_registrar::get_functions(interface_ident),
            events: tip_account_registrar::get_events(interface_ident),
            errors: tip_account_registrar::get_errors(interface_ident),
        }),

        // Test interfaces
        "ITestToken" => Ok(Interface {
            functions: tests::get_itest_token_functions(interface_ident),
            events: Vec::new(),
            errors: Vec::new(),
        }),
        "IMetadata" => Ok(Interface {
            functions: tests::get_imetadata_functions(interface_ident),
            events: Vec::new(),
            errors: Vec::new(),
        }),
        "IMiniToken" => Ok(Interface {
            functions: tests::get_imini_token_functions(interface_ident),
            events: tests::get_imini_token_events(interface_ident),
            errors: Vec::new(),
        }),
        "IErrorTest" => Ok(Interface {
            functions: tests::get_ierror_test_functions(interface_ident),
            events: Vec::new(),
            errors: tests::get_ierror_test_errors(interface_ident),
        }),

        // Unknown interface
        _ => {
            eprintln!(
                "Warning: Interface '{interface_name}' not in registry. No trait methods will be generated."
            );
            Ok(Interface {
                functions: Vec::new(),
                events: Vec::new(),
                errors: Vec::new(),
            })
        }
    }
}
