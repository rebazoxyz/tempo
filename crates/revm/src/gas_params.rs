use auto_impl::auto_impl;
use revm::context_interface::cfg::{GasId, GasParams};

/// Extending [`GasParams`] for Tempo use case.
#[auto_impl(&, Arc, Box, &mut)]
pub trait TempoGasParams {
    fn gas_params(&self) -> &GasParams;

    fn tx_tip1000_auth_account_creation_cost(&self) -> u64 {
        self.gas_params().get(GasId::new(255))
    }
}

impl TempoGasParams for GasParams {
    fn gas_params(&self) -> &GasParams {
        self
    }
}
