use crate::interface::{InterfaceError, InterfaceEvent, InterfaceFunction, ParamName};
use quote::quote;
use syn::{Ident, Type, parse_quote};

pub(crate) fn get_functions(interface_ident: &Ident) -> Vec<InterfaceFunction> {
    // Helper to convert parameter tuples to ParamName
    let params = |p: Vec<(&'static str, Type)>| -> Vec<(ParamName, Type)> {
        p.into_iter()
            .map(|(name, ty)| (ParamName::new(name), ty))
            .collect()
    };

    vec![
        // Metadata functions (view, no parameters)
        InterfaceFunction {
            name: "name",
            params: vec![],
            return_type: parse_quote!(String),
            is_view: true,
            call_type_path: quote!(#interface_ident::nameCall),
        },
        InterfaceFunction {
            name: "symbol",
            params: vec![],
            return_type: parse_quote!(String),
            is_view: true,
            call_type_path: quote!(#interface_ident::symbolCall),
        },
        InterfaceFunction {
            name: "decimals",
            params: vec![],
            return_type: parse_quote!(u8),
            is_view: true,
            call_type_path: quote!(#interface_ident::decimalsCall),
        },
        InterfaceFunction {
            name: "currency",
            params: vec![],
            return_type: parse_quote!(String),
            is_view: true,
            call_type_path: quote!(#interface_ident::currencyCall),
        },
        InterfaceFunction {
            name: "total_supply",
            params: vec![],
            return_type: parse_quote!(U256),
            is_view: true,
            call_type_path: quote!(#interface_ident::totalSupplyCall),
        },
        InterfaceFunction {
            name: "supply_cap",
            params: vec![],
            return_type: parse_quote!(U256),
            is_view: true,
            call_type_path: quote!(#interface_ident::supplyCapCall),
        },
        InterfaceFunction {
            name: "transfer_policy_id",
            params: vec![],
            return_type: parse_quote!(u64),
            is_view: true,
            call_type_path: quote!(#interface_ident::transferPolicyIdCall),
        },
        InterfaceFunction {
            name: "paused",
            params: vec![],
            return_type: parse_quote!(bool),
            is_view: true,
            call_type_path: quote!(#interface_ident::pausedCall),
        },
        InterfaceFunction {
            name: "quote_token",
            params: vec![],
            return_type: parse_quote!(Address),
            is_view: true,
            call_type_path: quote!(#interface_ident::quoteTokenCall),
        },
        InterfaceFunction {
            name: "next_quote_token",
            params: vec![],
            return_type: parse_quote!(Address),
            is_view: true,
            call_type_path: quote!(#interface_ident::nextQuoteTokenCall),
        },
        // View functions with parameters
        InterfaceFunction {
            name: "balance_of",
            params: params(vec![("account", parse_quote!(Address))]),
            return_type: parse_quote!(U256),
            is_view: true,
            call_type_path: quote!(#interface_ident::balanceOfCall),
        },
        InterfaceFunction {
            name: "allowance",
            params: params(vec![
                ("owner", parse_quote!(Address)),
                ("spender", parse_quote!(Address)),
            ]),
            return_type: parse_quote!(U256),
            is_view: true,
            call_type_path: quote!(#interface_ident::allowanceCall),
        },
        // Mutating functions (non-void)
        InterfaceFunction {
            name: "transfer",
            params: params(vec![
                ("to", parse_quote!(Address)),
                ("amount", parse_quote!(U256)),
            ]),
            return_type: parse_quote!(bool),
            is_view: false,
            call_type_path: quote!(#interface_ident::transferCall),
        },
        InterfaceFunction {
            name: "transfer_from",
            params: params(vec![
                ("from", parse_quote!(Address)),
                ("to", parse_quote!(Address)),
                ("amount", parse_quote!(U256)),
            ]),
            return_type: parse_quote!(bool),
            is_view: false,
            call_type_path: quote!(#interface_ident::transferFromCall),
        },
        InterfaceFunction {
            name: "approve",
            params: params(vec![
                ("spender", parse_quote!(Address)),
                ("amount", parse_quote!(U256)),
            ]),
            return_type: parse_quote!(bool),
            is_view: false,
            call_type_path: quote!(#interface_ident::approveCall),
        },
        InterfaceFunction {
            name: "transfer_from_with_memo",
            params: params(vec![
                ("from", parse_quote!(Address)),
                ("to", parse_quote!(Address)),
                ("amount", parse_quote!(U256)),
                ("memo", parse_quote!(B256)),
            ]),
            return_type: parse_quote!(bool),
            is_view: false,
            call_type_path: quote!(#interface_ident::transferFromWithMemoCall),
        },
        // Mutating functions (void)
        InterfaceFunction {
            name: "mint",
            params: params(vec![
                ("to", parse_quote!(Address)),
                ("amount", parse_quote!(U256)),
            ]),
            return_type: parse_quote!(()),
            is_view: false,
            call_type_path: quote!(#interface_ident::mintCall),
        },
        InterfaceFunction {
            name: "burn",
            params: params(vec![("amount", parse_quote!(U256))]),
            return_type: parse_quote!(()),
            is_view: false,
            call_type_path: quote!(#interface_ident::burnCall),
        },
        InterfaceFunction {
            name: "mint_with_memo",
            params: params(vec![
                ("to", parse_quote!(Address)),
                ("amount", parse_quote!(U256)),
                ("memo", parse_quote!(B256)),
            ]),
            return_type: parse_quote!(()),
            is_view: false,
            call_type_path: quote!(#interface_ident::mintWithMemoCall),
        },
        InterfaceFunction {
            name: "burn_with_memo",
            params: params(vec![
                ("amount", parse_quote!(U256)),
                ("memo", parse_quote!(B256)),
            ]),
            return_type: parse_quote!(()),
            is_view: false,
            call_type_path: quote!(#interface_ident::burnWithMemoCall),
        },
        InterfaceFunction {
            name: "burn_blocked",
            params: params(vec![
                ("from", parse_quote!(Address)),
                ("amount", parse_quote!(U256)),
            ]),
            return_type: parse_quote!(()),
            is_view: false,
            call_type_path: quote!(#interface_ident::burnBlockedCall),
        },
        InterfaceFunction {
            name: "transfer_with_memo",
            params: params(vec![
                ("to", parse_quote!(Address)),
                ("amount", parse_quote!(U256)),
                ("memo", parse_quote!(B256)),
            ]),
            return_type: parse_quote!(()),
            is_view: false,
            call_type_path: quote!(#interface_ident::transferWithMemoCall),
        },
        // Admin functions (void)
        InterfaceFunction {
            name: "change_transfer_policy_id",
            params: params(vec![("newPolicyId", parse_quote!(u64))]),
            return_type: parse_quote!(()),
            is_view: false,
            call_type_path: quote!(#interface_ident::changeTransferPolicyIdCall),
        },
        InterfaceFunction {
            name: "set_supply_cap",
            params: params(vec![("newSupplyCap", parse_quote!(U256))]),
            return_type: parse_quote!(()),
            is_view: false,
            call_type_path: quote!(#interface_ident::setSupplyCapCall),
        },
        InterfaceFunction {
            name: "pause",
            params: vec![],
            return_type: parse_quote!(()),
            is_view: false,
            call_type_path: quote!(#interface_ident::pauseCall),
        },
        InterfaceFunction {
            name: "unpause",
            params: vec![],
            return_type: parse_quote!(()),
            is_view: false,
            call_type_path: quote!(#interface_ident::unpauseCall),
        },
        InterfaceFunction {
            name: "update_quote_token",
            params: params(vec![("newQuoteToken", parse_quote!(Address))]),
            return_type: parse_quote!(()),
            is_view: false,
            call_type_path: quote!(#interface_ident::updateQuoteTokenCall),
        },
        InterfaceFunction {
            name: "finalize_quote_token_update",
            params: vec![],
            return_type: parse_quote!(()),
            is_view: false,
            call_type_path: quote!(#interface_ident::finalizeQuoteTokenUpdateCall),
        },
    ]
}

pub(crate) fn get_events(interface_ident: &Ident) -> Vec<InterfaceEvent> {
    vec![
        // Core token events
        InterfaceEvent {
            name: "transfer",
            params: vec![
                ("from", parse_quote!(Address), true),
                ("to", parse_quote!(Address), true),
                ("amount", parse_quote!(U256), false),
            ],
            event_type_path: quote!(#interface_ident::Transfer),
        },
        InterfaceEvent {
            name: "approval",
            params: vec![
                ("owner", parse_quote!(Address), true),
                ("spender", parse_quote!(Address), true),
                ("amount", parse_quote!(U256), false),
            ],
            event_type_path: quote!(#interface_ident::Approval),
        },
        InterfaceEvent {
            name: "mint",
            params: vec![
                ("to", parse_quote!(Address), true),
                ("amount", parse_quote!(U256), false),
            ],
            event_type_path: quote!(#interface_ident::Mint),
        },
        InterfaceEvent {
            name: "burn",
            params: vec![
                ("from", parse_quote!(Address), true),
                ("amount", parse_quote!(U256), false),
            ],
            event_type_path: quote!(#interface_ident::Burn),
        },
        InterfaceEvent {
            name: "burn_blocked",
            params: vec![
                ("from", parse_quote!(Address), true),
                ("amount", parse_quote!(U256), false),
            ],
            event_type_path: quote!(#interface_ident::BurnBlocked),
        },
        InterfaceEvent {
            name: "transfer_with_memo",
            params: vec![
                ("from", parse_quote!(Address), true),
                ("to", parse_quote!(Address), true),
                ("amount", parse_quote!(U256), false),
                ("memo", parse_quote!(B256), false),
            ],
            event_type_path: quote!(#interface_ident::TransferWithMemo),
        },
        // Admin events
        InterfaceEvent {
            name: "transfer_policy_update",
            params: vec![
                ("updater", parse_quote!(Address), true),
                ("newPolicyId", parse_quote!(u64), true),
            ],
            event_type_path: quote!(#interface_ident::TransferPolicyUpdate),
        },
        InterfaceEvent {
            name: "supply_cap_update",
            params: vec![
                ("updater", parse_quote!(Address), true),
                ("newSupplyCap", parse_quote!(U256), true),
            ],
            event_type_path: quote!(#interface_ident::SupplyCapUpdate),
        },
        InterfaceEvent {
            name: "pause_state_update",
            params: vec![
                ("updater", parse_quote!(Address), true),
                ("isPaused", parse_quote!(bool), false),
            ],
            event_type_path: quote!(#interface_ident::PauseStateUpdate),
        },
        InterfaceEvent {
            name: "update_quote_token",
            params: vec![
                ("updater", parse_quote!(Address), true),
                ("newQuoteToken", parse_quote!(Address), true),
            ],
            event_type_path: quote!(#interface_ident::UpdateQuoteToken),
        },
        InterfaceEvent {
            name: "quote_token_update_finalized",
            params: vec![
                ("updater", parse_quote!(Address), true),
                ("newQuoteToken", parse_quote!(Address), true),
            ],
            event_type_path: quote!(#interface_ident::QuoteTokenUpdateFinalized),
        },
    ]
}

pub(crate) fn get_errors(interface_ident: &Ident) -> Vec<InterfaceError> {
    vec![
        // Balance and allowance errors
        InterfaceError {
            name: "insufficient_balance",
            params: vec![],
            error_type_path: quote!(#interface_ident::InsufficientBalance),
        },
        InterfaceError {
            name: "insufficient_allowance",
            params: vec![],
            error_type_path: quote!(#interface_ident::InsufficientAllowance),
        },
        // Supply errors
        InterfaceError {
            name: "supply_cap_exceeded",
            params: vec![],
            error_type_path: quote!(#interface_ident::SupplyCapExceeded),
        },
        // Payload errors
        InterfaceError {
            name: "invalid_payload",
            params: vec![],
            error_type_path: quote!(#interface_ident::InvalidPayload),
        },
        InterfaceError {
            name: "string_too_long",
            params: vec![],
            error_type_path: quote!(#interface_ident::StringTooLong),
        },
        // Transfer policy errors
        InterfaceError {
            name: "policy_forbids",
            params: vec![],
            error_type_path: quote!(#interface_ident::PolicyForbids),
        },
        // Address errors
        InterfaceError {
            name: "invalid_recipient",
            params: vec![],
            error_type_path: quote!(#interface_ident::InvalidRecipient),
        },
        // State errors
        InterfaceError {
            name: "contract_paused",
            params: vec![],
            error_type_path: quote!(#interface_ident::ContractPaused),
        },
        // Currency errors
        InterfaceError {
            name: "invalid_currency",
            params: vec![],
            error_type_path: quote!(#interface_ident::InvalidCurrency),
        },
        // Quote token errors
        InterfaceError {
            name: "invalid_quote_token",
            params: vec![],
            error_type_path: quote!(#interface_ident::InvalidQuoteToken),
        },
        // Transfer errors
        InterfaceError {
            name: "transfers_disabled",
            params: vec![],
            error_type_path: quote!(#interface_ident::TransfersDisabled),
        },
        // Amount errors
        InterfaceError {
            name: "invalid_amount",
            params: vec![],
            error_type_path: quote!(#interface_ident::InvalidAmount),
        },
        // Access control errors
        InterfaceError {
            name: "unauthorized",
            params: vec![],
            error_type_path: quote!(#interface_ident::Unauthorized),
        },
    ]
}
