use alloy::primitives::{Address, B256};

use crate::{error::Result, storage::PrecompileStorageProvider, tip20_migrated::RolesAuthError};

use super::{TIP20Token, TIP20Token_IRolesAuth};

/// The default admin role identifier (zero bytes)
pub const DEFAULT_ADMIN_ROLE: B256 = B256::ZERO;
/// Role that cannot be granted to prevent security issues
pub const UNGRANTABLE_ROLE: B256 = B256::new([0xff; 32]);

impl<'a, S: PrecompileStorageProvider> TIP20Token_IRolesAuth for TIP20Token<'a, S> {
    /// Grants a role to an account (requires admin)
    fn grant_role(&mut self, msg_sender: Address, role: B256, account: Address) -> Result<()> {
        let admin_role = self.get_role_admin_internal(role)?;
        self.check_role_internal(msg_sender, admin_role)?;
        self.grant_role_internal(account, role)?;
        self.emit_role_membership_updated(role, account, msg_sender, true)
    }

    /// Revokes a role from an account (requires admin)
    fn revoke_role(&mut self, msg_sender: Address, role: B256, account: Address) -> Result<()> {
        let admin_role = self.get_role_admin_internal(role)?;
        self.check_role_internal(msg_sender, admin_role)?;
        self.revoke_role_internal(account, role)?;
        self.emit_role_membership_updated(role, account, msg_sender, false)
    }

    /// Allows caller to renounce their own role
    fn renounce_role(&mut self, msg_sender: Address, role: B256) -> Result<()> {
        self.check_role_internal(msg_sender, role)?;
        self.revoke_role_internal(msg_sender, role)?;
        self.emit_role_membership_updated(role, msg_sender, msg_sender, false)
    }

    /// Sets the admin role for a given role
    fn set_role_admin(&mut self, msg_sender: Address, role: B256, admin_role: B256) -> Result<()> {
        let current_admin_role = self.get_role_admin_internal(role)?;
        self.check_role_internal(msg_sender, current_admin_role)?;

        self.set_role_admin_internal(role, admin_role)?;
        self.emit_role_admin_updated(role, admin_role, msg_sender)
    }
}

impl<'a, S: PrecompileStorageProvider> TIP20Token<'a, S> {
    /// Initialize the UNGRANTABLE_ROLE to be self-administered
    pub(crate) fn roles_initialize(&mut self) -> Result<()> {
        self.set_role_admin_internal(UNGRANTABLE_ROLE, UNGRANTABLE_ROLE)
    }

    /// Grant the default admin role to an account
    pub fn roles_grant_default_admin(&mut self, admin: Address) -> Result<()> {
        self.grant_role_internal(admin, DEFAULT_ADMIN_ROLE)
    }

    // Utility functions for checking roles without calldata
    /// Reverts if account does not have the specified role
    pub fn check_role(&mut self, account: Address, role: B256) -> Result<()> {
        self.check_role_internal(account, role)
    }

    /// Internal function to grant a role without permission checks
    pub fn grant_role_internal(&mut self, account: Address, role: B256) -> Result<()> {
        self.sstore_roles(account, role, true)
    }

    /// Internal function to revoke a role without permission checks
    fn revoke_role_internal(&mut self, account: Address, role: B256) -> Result<()> {
        self.sstore_roles(account, role, false)
    }

    /// Internal function to get the admin role for a given role
    fn get_role_admin_internal(&mut self, role: B256) -> Result<B256> {
        let admin = self.sload_role_admins(role)?;
        Ok(B256::from(admin)) // If sloads 0, will be equal to DEFAULT_ADMIN_ROLE
    }

    /// Internal function to set the admin role without permission checks
    fn set_role_admin_internal(&mut self, role: B256, admin_role: B256) -> Result<()> {
        self.sstore_role_admins(role, admin_role)
    }

    /// Internal function to check role membership without calldata
    fn check_role_internal(&mut self, account: Address, role: B256) -> Result<()> {
        if !self.sload_roles(account, role)? {
            return Err(RolesAuthError::unauthorized().into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloy::primitives::{Address, keccak256};

    use super::{DEFAULT_ADMIN_ROLE, TIP20Token_IRolesAuth};
    use crate::{
        storage::hashmap::HashMapStorageProvider,
        tip20_migrated::{LINKING_USD_ADDRESS, RolesAuthError, TIP20Token},
    };

    #[test]
    fn test_role_contract_grant_and_check() -> eyre::Result<()> {
        let mut storage = HashMapStorageProvider::new(1);
        let token_id = 1;

        let admin = Address::from([1u8; 20]);
        let user = Address::from([2u8; 20]);
        let custom_role = keccak256(b"CUSTOM_ROLE");

        {
            let mut token = TIP20Token::new(token_id, &mut storage);

            // Initialize token with admin
            token.initialize("Test", "TST", "USD", LINKING_USD_ADDRESS, admin)?;

            // Test admin has DEFAULT_ADMIN_ROLE
            let has_admin = token.sload_roles(admin, DEFAULT_ADMIN_ROLE)?;
            assert!(has_admin);

            // Grant custom role to user
            token.grant_role(admin, custom_role, user)?;

            // Check custom role was granted
            let has_custom = token.sload_roles(user, custom_role)?;
            assert!(has_custom);
        }

        Ok(())
    }

    #[test]
    fn test_role_admin_functions() -> eyre::Result<()> {
        let mut storage = HashMapStorageProvider::new(1);
        let token_id = 1;

        let admin = Address::from([1u8; 20]);
        let custom_role = keccak256(b"CUSTOM_ROLE");
        let admin_role = keccak256(b"ADMIN_ROLE");

        {
            let mut token = TIP20Token::new(token_id, &mut storage);

            // Initialize and grant default admin
            token.initialize("Test", "TST", "USD", LINKING_USD_ADDRESS, admin)?;

            // Set custom admin for role
            token.set_role_admin(admin, custom_role, admin_role)?;

            // Check role admin was set
            let retrieved_admin = token.get_role_admin_internal(custom_role)?;
            assert_eq!(retrieved_admin, admin_role);
        }

        Ok(())
    }

    #[test]
    fn test_renounce_role() -> eyre::Result<()> {
        let mut storage = HashMapStorageProvider::new(1);
        let token_id = 1;

        let user = Address::from([1u8; 20]);
        let custom_role = keccak256(b"CUSTOM_ROLE");

        {
            let mut token = TIP20Token::new(token_id, &mut storage);

            // Initialize token (doesn't matter who admin is for this test)
            token.initialize("Test", "TST", "USD", LINKING_USD_ADDRESS, user)?;

            // Grant role internally
            token.grant_role_internal(user, custom_role)?;

            // Verify role was granted
            assert!(token.sload_roles(user, custom_role)?);

            // Renounce role
            token.renounce_role(user, custom_role)?;

            // Check role is removed
            assert!(!token.sload_roles(user, custom_role)?);
        }

        Ok(())
    }

    #[test]
    fn test_unauthorized_access() -> eyre::Result<()> {
        let mut storage = HashMapStorageProvider::new(1);
        let token_id = 1;

        let admin = Address::from([1u8; 20]);
        let user = Address::from([2u8; 20]);
        let other = Address::from([3u8; 20]);
        let custom_role = keccak256(b"CUSTOM_ROLE");

        {
            let mut token = TIP20Token::new(token_id, &mut storage);

            // Initialize with admin
            token.initialize("Test", "TST", "USD", LINKING_USD_ADDRESS, admin)?;

            // Try to grant role without permission (user is not admin)
            let result = token.grant_role(user, custom_role, other);

            // Should fail with Unauthorized error
            assert!(result.is_err());
            let err = result.unwrap_err();

            // Check it's a RolesAuthError::Unauthorized
            match err {
                crate::error::TempoPrecompileError::RolesAuthError(
                    RolesAuthError::Unauthorized(_),
                ) => {
                    // Expected error
                }
                other => panic!("Expected RolesAuthError::Unauthorized, got: {:?}", other),
            }
        }

        Ok(())
    }
}
