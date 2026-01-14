//! TIP20 Solidity ABI type definitions.
//!
//! This module contains all `#[solidity]` macro definitions for the TIP20 token system,
//! organized into three separate interfaces:
//! - `tip20`: Core ERC20-like token interface
//! - `roles_auth`: Role-based access control interface
//! - `rewards`: Reward distribution and claiming interface

use alloy::primitives::{B256, U256};
use tempo_precompiles_macros::solidity;

#[solidity]
pub mod tip20 {
    use alloy::primitives::{Address, B256, U256};

    use crate::error::Result;

    pub enum Error {
        InsufficientBalance {
            available: U256,
            required: U256,
            token: Address,
        },
        InsufficientAllowance,
        SupplyCapExceeded,
        InvalidSupplyCap,
        InvalidPayload,
        StringTooLong,
        PolicyForbids,
        InvalidRecipient,
        ContractPaused,
        InvalidCurrency,
        InvalidQuoteToken,
        TransfersDisabled,
        InvalidAmount,
        NoOptedInSupply,
        ProtectedAddress,
        InvalidToken,
        Uninitialized,
        InvalidTransferPolicyId,
    }

    pub enum Event {
        Transfer {
            #[indexed]
            from: Address,
            #[indexed]
            to: Address,
            amount: U256,
        },
        Approval {
            #[indexed]
            owner: Address,
            #[indexed]
            spender: Address,
            amount: U256,
        },
        Mint {
            #[indexed]
            to: Address,
            amount: U256,
        },
        Burn {
            #[indexed]
            from: Address,
            amount: U256,
        },
        BurnBlocked {
            #[indexed]
            from: Address,
            amount: U256,
        },
        TransferWithMemo {
            #[indexed]
            from: Address,
            #[indexed]
            to: Address,
            amount: U256,
            #[indexed]
            memo: B256,
        },
        TransferPolicyUpdate {
            #[indexed]
            updater: Address,
            #[indexed]
            new_policy_id: u64,
        },
        SupplyCapUpdate {
            #[indexed]
            updater: Address,
            #[indexed]
            new_supply_cap: U256,
        },
        PauseStateUpdate {
            #[indexed]
            updater: Address,
            is_paused: bool,
        },
        NextQuoteTokenSet {
            #[indexed]
            updater: Address,
            #[indexed]
            next_quote_token: Address,
        },
        QuoteTokenUpdate {
            #[indexed]
            updater: Address,
            #[indexed]
            new_quote_token: Address,
        },
    }

    pub trait Interface {
        // View functions
        fn name(&self) -> Result<String>;
        fn symbol(&self) -> Result<String>;
        fn decimals(&self) -> Result<u8>;
        fn total_supply(&self) -> Result<U256>;
        fn quote_token(&self) -> Result<Address>;
        fn next_quote_token(&self) -> Result<Address>;
        fn balance_of(&self, account: Address) -> Result<U256>;
        fn allowance(&self, owner: Address, spender: Address) -> Result<U256>;
        fn currency(&self) -> Result<String>;
        fn supply_cap(&self) -> Result<U256>;
        fn paused(&self) -> Result<bool>;
        fn transfer_policy_id(&self) -> Result<u64>;
        fn PAUSE_ROLE(&self) -> Result<B256>;
        fn UNPAUSE_ROLE(&self) -> Result<B256>;
        fn ISSUER_ROLE(&self) -> Result<B256>;
        fn BURN_BLOCKED_ROLE(&self) -> Result<B256>;

        // Mutating functions
        fn transfer(&mut self, to: Address, amount: U256) -> Result<bool>;
        fn approve(&mut self, spender: Address, amount: U256) -> Result<bool>;
        fn transfer_from(&mut self, from: Address, to: Address, amount: U256) -> Result<bool>;
        fn mint(&mut self, to: Address, amount: U256) -> Result<()>;
        fn burn(&mut self, amount: U256) -> Result<()>;
        fn burn_blocked(&mut self, from: Address, amount: U256) -> Result<()>;
        fn mint_with_memo(&mut self, to: Address, amount: U256, memo: B256) -> Result<()>;
        fn burn_with_memo(&mut self, amount: U256, memo: B256) -> Result<()>;
        fn transfer_with_memo(&mut self, to: Address, amount: U256, memo: B256) -> Result<()>;
        fn transfer_from_with_memo(
            &mut self,
            from: Address,
            to: Address,
            amount: U256,
            memo: B256,
        ) -> Result<bool>;
        fn change_transfer_policy_id(&mut self, new_policy_id: u64) -> Result<()>;
        fn set_supply_cap(&mut self, new_supply_cap: U256) -> Result<()>;
        fn pause(&mut self) -> Result<()>;
        fn unpause(&mut self) -> Result<()>;
        fn set_next_quote_token(&mut self, new_quote_token: Address) -> Result<()>;
        fn complete_quote_token_update(&mut self) -> Result<()>;
    }
}

#[solidity]
pub mod roles_auth {
    use alloy::primitives::{Address, B256};

    use crate::error::Result;

    pub enum Error {
        Unauthorized,
    }

    pub enum Event {
        RoleMembershipUpdated {
            #[indexed]
            role: B256,
            #[indexed]
            account: Address,
            #[indexed]
            sender: Address,
            has_role: bool,
        },
        RoleAdminUpdated {
            #[indexed]
            role: B256,
            #[indexed]
            new_admin_role: B256,
            #[indexed]
            sender: Address,
        },
    }

    pub trait Interface {
        fn has_role(&self, account: Address, role: B256) -> Result<bool>;
        fn get_role_admin(&self, role: B256) -> Result<B256>;
        fn grant_role(&mut self, role: B256, account: Address) -> Result<()>;
        fn revoke_role(&mut self, role: B256, account: Address) -> Result<()>;
        fn renounce_role(&mut self, role: B256) -> Result<()>;
        fn set_role_admin(&mut self, role: B256, admin_role: B256) -> Result<()>;
    }
}

pub const ACC_PRECISION: U256 = alloy::primitives::uint!(1000000000000000000_U256);

#[solidity]
pub mod rewards {
    use alloy::primitives::{Address, U256};
    use tempo_precompiles_macros::Storable;

    use crate::error::Result;

    #[derive(Debug, Clone, PartialEq, Eq, Storable)]
    pub struct UserRewardInfo {
        pub reward_recipient: Address,
        pub reward_per_token: U256,
        pub reward_balance: U256,
    }

    pub enum Event {
        RewardDistributed {
            #[indexed]
            funder: Address,
            amount: U256,
        },
        RewardRecipientSet {
            #[indexed]
            holder: Address,
            #[indexed]
            recipient: Address,
        },
    }

    pub trait Interface {
        fn distribute_reward(&mut self, amount: U256) -> Result<()>;
        fn set_reward_recipient(&mut self, recipient: Address) -> Result<()>;
        fn claim_rewards(&mut self) -> Result<U256>;
        fn opted_in_supply(&self) -> Result<u128>;
        fn global_reward_per_token(&self) -> Result<U256>;
        fn user_reward_info(&self, account: Address) -> Result<UserRewardInfo>;
        fn get_pending_rewards(&self, account: Address) -> Result<u128>;
    }
}

pub use rewards::UserRewardInfo;

pub const DEFAULT_ADMIN_ROLE: B256 = B256::ZERO;
pub const UNGRANTABLE_ROLE: B256 = B256::new([0xff; 32]);

// Backward-compatibility aliases for Error/Event types
pub use tip20::Error as TIP20Error;
pub use tip20::Event as TIP20Event;
pub use tip20::Calls as TIP20Calls;
pub use tip20::Interface as ITIP20Interface;
pub use tip20::{InvalidCurrency, PolicyForbids};
pub use roles_auth::Error as RolesAuthError;
pub use roles_auth::Event as RolesAuthEvent;

// Backward-compatibility module alias (ITIP20 -> ITip20)
pub use ITip20 as ITIP20;
