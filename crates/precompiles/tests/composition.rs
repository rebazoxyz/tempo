//! Tests for `#[contract(solidity(...))]` composition feature.

pub mod storage_primitives {
    pub use tempo_precompiles::storage::*;
}
pub use storage_primitives as storage;
pub use tempo_precompiles::error;

use alloy::{
    primitives::{Address, B256, IntoLogData, U256},
    sol_types::{SolCall, SolInterface},
};
use tempo_precompiles::tip20::types::{rewards, roles_auth, tip20};
use tempo_precompiles_macros::contract;

#[contract(abi(tip20, roles_auth, rewards))]
pub struct TestComposedContract {
    value: U256,
}

#[test]
fn test_composed_calls_enum_decode() {
    let call = tip20::balanceOfCall {
        account: Address::random(),
    };
    let encoded = <tip20::Calls as SolInterface>::abi_encode(&call.into());

    let decoded = TestComposedContractCalls::abi_decode(&encoded).unwrap();
    assert!(matches!(
        decoded,
        TestComposedContractCalls::Tip20(tip20::Calls::balanceOf(_))
    ));

    let call = roles_auth::hasRoleCall {
        role: B256::random(),
        account: Address::random(),
    };
    let encoded = <roles_auth::Calls as SolInterface>::abi_encode(&call.into());

    let decoded = TestComposedContractCalls::abi_decode(&encoded).unwrap();
    assert!(matches!(
        decoded,
        TestComposedContractCalls::RolesAuth(roles_auth::Calls::hasRole(_))
    ));
}

#[test]
fn test_composed_calls_selectors_flattened() {
    assert!(!TestComposedContractCalls::SELECTORS.is_empty());
    assert_eq!(
        TestComposedContractCalls::SELECTORS.len(),
        tip20::Calls::SELECTORS.len()
            + roles_auth::Calls::SELECTORS.len()
            + rewards::Calls::SELECTORS.len()
    );

    for selector in tip20::Calls::SELECTORS {
        assert!(TestComposedContractCalls::valid_selector(*selector));
    }
    for selector in roles_auth::Calls::SELECTORS {
        assert!(TestComposedContractCalls::valid_selector(*selector));
    }
    for selector in rewards::Calls::SELECTORS {
        assert!(TestComposedContractCalls::valid_selector(*selector));
    }
}

#[test]
fn test_composed_error_from_impls() {
    let err: TestComposedContractError =
        tip20::Error::insufficient_balance(U256::from(100), U256::from(200), Address::random())
            .into();
    assert!(matches!(err, TestComposedContractError::Tip20(_)));

    let err: TestComposedContractError = roles_auth::Error::unauthorized().into();
    assert!(matches!(err, TestComposedContractError::RolesAuth(_)));

    assert_eq!(
        TestComposedContractError::SELECTORS.len(),
        tip20::Error::SELECTORS.len() + roles_auth::Error::SELECTORS.len()
    );
}

#[test]
fn test_composed_event_from_impls() {
    let event: TestComposedContractEvent =
        tip20::Event::transfer(Address::random(), Address::random(), U256::from(100)).into();
    assert!(matches!(event, TestComposedContractEvent::Tip20(_)));
    let log_data = event.into_log_data();
    assert!(!log_data.topics().is_empty());

    let event: TestComposedContractEvent = roles_auth::Event::role_membership_updated(
        B256::random(),
        Address::random(),
        Address::random(),
        true,
    )
    .into();
    assert!(matches!(event, TestComposedContractEvent::RolesAuth(_)));

    assert_eq!(
        TestComposedContractEvent::SELECTORS.len(),
        tip20::Event::SELECTORS.len()
            + roles_auth::Event::SELECTORS.len()
            + rewards::Event::SELECTORS.len()
    );
}

#[test]
fn test_unknown_selector_returns_error() {
    let unknown_calldata = [0xde, 0xad, 0xbe, 0xef, 0x00, 0x00, 0x00, 0x00];
    let result = TestComposedContractCalls::abi_decode(&unknown_calldata);
    assert!(result.is_err());
}

#[test]
fn test_sol_interface_trait_methods() {
    let call = tip20::balanceOfCall {
        account: Address::random(),
    };
    let composed: TestComposedContractCalls = tip20::Calls::balanceOf(call.clone()).into();

    let selector = SolInterface::selector(&composed);
    assert_eq!(selector, tip20::balanceOfCall::SELECTOR);

    let encoded_size = SolInterface::abi_encoded_size(&composed);
    assert!(encoded_size > 0);
}

impl From<tip20::Calls> for TestComposedContractCalls {
    fn from(calls: tip20::Calls) -> Self {
        Self::Tip20(calls)
    }
}

impl From<roles_auth::Calls> for TestComposedContractCalls {
    fn from(calls: roles_auth::Calls) -> Self {
        Self::RolesAuth(calls)
    }
}

impl From<rewards::Calls> for TestComposedContractCalls {
    fn from(calls: rewards::Calls) -> Self {
        Self::Rewards(calls)
    }
}
