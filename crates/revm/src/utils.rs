//! Gas arithmetic utilities.

use crate::TempoInvalidTransaction;
use tempo_chainspec::hardfork::TempoHardfork;

/// Gas arithmetic mode for hardfork-gated overflow checking.
///
/// This encapsulates the hardfork check so callers don't need to pass `TempoHardfork` everywhere.
#[derive(Debug, Clone, Copy, Default)]
pub enum GasMode {
    #[default]
    Checked,
    Unchecked, // Genesis used unchecked gas calculations
}

impl GasMode {
    /// Create a new `GasMode` from the current hardfork.
    #[inline]
    pub fn new(spec: TempoHardfork) -> Self {
        if spec.is_t0() {
            Self::Checked
        } else {
            Self::Unchecked
        }
    }

    /// Checked addition with hardfork-gated overflow behavior.
    #[inline]
    pub fn add(self, lhs: u64, rhs: u64) -> Result<u64, TempoInvalidTransaction> {
        match self {
            Self::Checked => lhs
                .checked_add(rhs)
                .ok_or(TempoInvalidTransaction::GasArithmeticOverflow),
            Self::Unchecked => Ok(lhs + rhs),
        }
    }

    /// Checked subtraction with hardfork-gated overflow behavior.
    #[inline]
    pub fn sub(self, lhs: u64, rhs: u64) -> Result<u64, TempoInvalidTransaction> {
        match self {
            Self::Checked => lhs
                .checked_sub(rhs)
                .ok_or(TempoInvalidTransaction::GasArithmeticOverflow),
            Self::Unchecked => Ok(lhs - rhs),
        }
    }

    /// Checked multiplication with hardfork-gated overflow behavior.
    #[inline]
    pub fn mul(self, lhs: u64, rhs: u64) -> Result<u64, TempoInvalidTransaction> {
        match self {
            Self::Checked => lhs
                .checked_mul(rhs)
                .ok_or(TempoInvalidTransaction::GasArithmeticOverflow),
            Self::Unchecked => Ok(lhs * rhs),
        }
    }

    /// Start a chainable gas calculation with fluent arithmetic that reads left-to-right.
    /// ```ignore
    /// let result: u64 = gas_mode
    ///     .calc(initial)
    ///     .add(a)?
    ///     .sub(b)?
    ///     .into();
    /// ```
    #[inline]
    pub const fn calc(self, value: u64) -> GasCalc {
        GasCalc { value, mode: self }
    }
}

/// Builder for chaining gas arithmetic operations.
///
/// Created via [`GasMode::calc`], enables fluent arithmetic that reads left-to-right.
#[derive(Debug, Clone, Copy)]
pub struct GasCalc {
    value: u64,
    mode: GasMode,
}

impl From<GasCalc> for u64 {
    #[inline]
    fn from(calc: GasCalc) -> Self {
        calc.value
    }
}

impl<E> From<GasCalc> for Result<u64, E> {
    #[inline]
    fn from(calc: GasCalc) -> Self {
        Ok(calc.value)
    }
}

#[allow(clippy::should_implement_trait)]
impl GasCalc {
    /// Checked addition.
    #[inline]
    pub fn add(self, rhs: u64) -> Result<Self, TempoInvalidTransaction> {
        Ok(Self {
            value: self.mode.add(self.value, rhs)?,
            mode: self.mode,
        })
    }

    /// Checked subtraction.
    #[inline]
    pub fn sub(self, rhs: u64) -> Result<Self, TempoInvalidTransaction> {
        Ok(Self {
            value: self.mode.sub(self.value, rhs)?,
            mode: self.mode,
        })
    }

    /// Checked multiplication.
    #[inline]
    pub fn mul(self, rhs: u64) -> Result<Self, TempoInvalidTransaction> {
        Ok(Self {
            value: self.mode.mul(self.value, rhs)?,
            mode: self.mode,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gas_mode_checked() {
        // Checked mode: returns errors on overflow
        let mode = GasMode::Checked;
        assert!(mode.add(u64::MAX, 1).is_err());
        assert!(mode.sub(0, 1).is_err());
        assert!(mode.mul(u64::MAX, 2).is_err());

        // Normal operations succeed
        assert_eq!(mode.add(1, 2).unwrap(), 3);
        assert_eq!(mode.sub(5, 3).unwrap(), 2);
        assert_eq!(mode.mul(4, 5).unwrap(), 20);
    }

    #[test]
    fn test_gas_mode_from_hardfork() {
        // Pre-T0 should be `Unchecked`
        assert!(matches!(
            GasMode::new(TempoHardfork::Genesis),
            GasMode::Unchecked
        ));

        // Post-T0 should be `Checked`
        assert!(matches!(GasMode::new(TempoHardfork::T0), GasMode::Checked));
    }

    #[test]
    fn test_gas_calc_chaining() {
        let mode = GasMode::Checked;

        // Chain multiple operations
        let result: u64 = mode.calc(100).add(50).unwrap().sub(30).unwrap().into();
        assert_eq!(result, 120);

        // Multiplication in chain
        let result: u64 = mode.calc(10).mul(5).unwrap().add(10).unwrap().into();
        assert_eq!(result, 60);
    }

    #[test]
    fn test_gas_calc_overflow_checked() {
        let mode = GasMode::Checked;

        // Overflow on add
        assert!(mode.calc(u64::MAX).add(1).is_err());

        // Underflow on sub
        assert!(mode.calc(5).sub(10).is_err());

        // Overflow on mul
        assert!(mode.calc(u64::MAX).mul(2).is_err());
    }

    #[test]
    fn test_gas_calc_unchecked() {
        let mode = GasMode::Unchecked;

        // Normal operations work
        let result: u64 = mode.calc(100).add(50).unwrap().sub(30).unwrap().into();
        assert_eq!(result, 120);
    }

    #[test]
    fn test_gas_calc_into_result() {
        let mode = GasMode::Checked;

        // .into() can return Result<u64, _> in appropriate context
        let result: Result<u64, TempoInvalidTransaction> =
            mode.calc(100).add(50).unwrap().sub(30).unwrap().into();
        assert_eq!(result.unwrap(), 120);
    }
}
