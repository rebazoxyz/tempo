use crate::{
    error::Result,
    storage::{Mapping, Slot, thread_local::AddressGuard},
};
use alloy::primitives::{Address, U256};

pub mod slots {
    use alloy::primitives::U256;

    pub const TOTAL_SUPPLY: U256 = U256::ZERO;
    pub const BALANCES: U256 = U256::ONE;
}

pub struct ThreadLocalToken {
    _address_guard: crate::storage::thread_local::AddressGuard,
    total_supply: Slot<U256>,
    balances: Mapping<Address, U256>,
}

impl ThreadLocalToken {
    pub fn new(address: Address) -> Result<Self> {
        // automatically creates and manages `AddressGuard`
        let guard = AddressGuard::new(address)?;
        Ok(Self {
            _address_guard: guard,
            total_supply: Slot::new(slots::TOTAL_SUPPLY),
            balances: Mapping::new(slots::BALANCES),
        })
    }

    pub fn total_supply(&self) -> Result<U256> {
        self.total_supply.read_tl()
    }

    fn set_total_supply(&self, value: U256) -> Result<()> {
        self.total_supply.write_tl(value)
    }

    pub fn balance_of(&self, account: Address) -> Result<U256> {
        self.balances.at(account).read_tl()
    }

    fn set_balance(&self, account: Address, balance: U256) -> Result<()> {
        self.balances.at(account).write_tl(balance)
    }

    pub fn mint(&self, to: Address, amount: U256) -> Result<()> {
        let balance = self.balance_of(to)?;
        let supply = self.total_supply()?;

        self.set_balance(to, balance + amount)?;
        self.set_total_supply(supply + amount)?;

        Ok(())
    }

    pub fn transfer(&self, from: Address, to: Address, amount: U256) -> Result<()> {
        let from_balance = self.balance_of(from)?;
        let to_balance = self.balance_of(to)?;

        self.set_balance(from, from_balance - amount)?;
        self.set_balance(to, to_balance + amount)?;

        Ok(())
    }
}

pub mod rewards_slots {
    use alloy::primitives::U256;
    pub const REWARDS_POOL: U256 = U256::ZERO;
}

pub struct ThreadLocalRewards {
    _address_guard: AddressGuard,
    rewards_pool: Slot<U256>,
}

impl ThreadLocalRewards {
    pub fn new(address: Address) -> Result<Self> {
        // automatically creates and manages `AddressGuard`
        let guard = AddressGuard::new(address)?;
        Ok(Self {
            _address_guard: guard,
            rewards_pool: Slot::new(rewards_slots::REWARDS_POOL),
        })
    }

    pub fn distribute(&self, transfer_amount: U256) -> Result<()> {
        let pool = self.rewards_pool.read_tl()?;
        let reward = transfer_amount / U256::from(100);

        self.rewards_pool.write_tl(pool + reward)?;

        Ok(())
    }

    pub fn get_pool(&self) -> Result<U256> {
        self.rewards_pool.read_tl()
    }
}

const REWARDS_ADDRESS: Address = Address::new([0xEE; 20]);

impl ThreadLocalToken {
    pub fn transfer_with_rewards(&self, from: Address, to: Address, amount: U256) -> Result<()> {
        // uses token's address from self._address_guard
        self.transfer(from, to, amount)?;

        {
            // uses rewards' address from `rewards._address_guard`
            let rewards = ThreadLocalRewards::new(REWARDS_ADDRESS)?;
            rewards.distribute(amount)?;
        }

        // once rewards is dropped, its guard is also dropped and we switch back to `token._address_guard`

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{hashmap::HashMapStorageProvider, thread_local::StorageGuard};

    #[test]
    fn test_pure_thread_local() -> Result<()> {
        let mut storage = HashMapStorageProvider::new(1);
        // this would be set at the dispatcher (entry point)
        let _storage_guard = unsafe { StorageGuard::new(&mut storage) };

        let token_address = Address::new([0x01; 20]);
        let alice = Address::new([0xA1; 20]);
        let bob = Address::new([0xB0; 20]);

        let token = ThreadLocalToken::new(token_address)?;

        // mint
        token.mint(alice, U256::from(1000))?;
        assert_eq!(token.balance_of(alice)?, U256::from(1000));
        assert_eq!(token.total_supply()?, U256::from(1000));

        // transfer
        token.transfer(alice, bob, U256::from(100))?;
        assert_eq!(token.balance_of(alice)?, U256::from(900));
        assert_eq!(token.balance_of(bob)?, U256::from(100));

        Ok(())
    }

    #[test]
    fn test_cross_contract_calls() -> Result<()> {
        let mut storage = HashMapStorageProvider::new(1);
        // this would be set at the dispatcher (entry point)
        let _storage_guard = unsafe { StorageGuard::new(&mut storage) };

        let token_address = Address::new([0x01; 20]);
        let alice = Address::new([0xA1; 20]);
        let bob = Address::new([0xB0; 20]);

        let token = ThreadLocalToken::new(token_address)?;
        token.mint(alice, U256::from(1000))?;

        // transfer with rewards
        token.transfer_with_rewards(alice, bob, U256::from(100))?;
        assert_eq!(token.balance_of(alice)?, U256::from(900));
        assert_eq!(token.balance_of(bob)?, U256::from(100));

        // verify rewards were distributed
        {
            let rewards = ThreadLocalRewards::new(REWARDS_ADDRESS)?;
            let pool = rewards.get_pool()?;
            assert_eq!(pool, U256::from(1));
        }

        Ok(())
    }

    #[test]
    fn test_nested_call_depth() -> Result<()> {
        use crate::storage::thread_local::context;

        let mut storage = HashMapStorageProvider::new(1);
        let addr1 = Address::new([0x01; 20]);
        let addr2 = Address::new([0x02; 20]);
        let addr3 = Address::new([0x03; 20]);

        let _storage_guard = unsafe { StorageGuard::new(&mut storage) };

        // demonstrate nested contract instantiation and automatic address switching
        {
            let _token1 = ThreadLocalToken::new(addr1)?;
            assert_eq!(context::call_depth(), 1);

            {
                let _token2 = ThreadLocalToken::new(addr2)?;
                assert_eq!(context::call_depth(), 2);

                {
                    let _token3 = ThreadLocalToken::new(addr3)?;
                    assert_eq!(context::call_depth(), 3);
                }

                assert_eq!(context::call_depth(), 2);
            }

            assert_eq!(context::call_depth(), 1);
        }

        Ok(())
    }
}
