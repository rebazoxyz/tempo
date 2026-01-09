// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import { TIP20 } from "../../src/TIP20.sol";
import { BaseTest } from "../BaseTest.t.sol";
import { FeeManagerHandler } from "./FeeManagerHandler.sol";
import { StdInvariant } from "forge-std/StdInvariant.sol";
import { console } from "forge-std/console.sol";

/// @title FeeManagerInvariantTest
/// @notice Invariant tests for the FeeManager contract
/// @dev Tests run against both Solidity reference (forge test) and Rust precompiles (tempo-forge test)
contract FeeManagerInvariantTest is StdInvariant, BaseTest {

    FeeManagerHandler public handler;
    TIP20 public userToken;
    TIP20 public validatorToken;

    function setUp() public override {
        super.setUp();

        // Create tokens for testing
        userToken =
            TIP20(factory.createToken("UserToken", "UTK", "USD", pathUSD, admin, bytes32("user")));
        validatorToken = TIP20(
            factory.createToken(
                "ValidatorToken", "VTK", "USD", pathUSD, admin, bytes32("validator")
            )
        );

        // Grant issuer role for minting
        userToken.grantRole(_ISSUER_ROLE, admin);
        validatorToken.grantRole(_ISSUER_ROLE, admin);

        // Setup initial pool liquidity (required for different-token swaps)
        uint256 initialLiquidity = 10_000_000e18;
        validatorToken.mint(admin, initialLiquidity);
        validatorToken.approve(address(amm), initialLiquidity);
        amm.mint(address(userToken), address(validatorToken), initialLiquidity, admin);

        // Create handler with admin for minting tokens
        handler = new FeeManagerHandler(amm, userToken, validatorToken, admin);

        // Grant issuer role to handler so it can mint tokens directly
        userToken.grantRole(_ISSUER_ROLE, address(handler));
        validatorToken.grantRole(_ISSUER_ROLE, address(handler));

        // Target only the handler
        targetContract(address(handler));
    }

    /*//////////////////////////////////////////////////////////////
                INVARIANT F1: FEES COLLECTED <= FEES IN
    //////////////////////////////////////////////////////////////*/

    /// @notice Total collected fees cannot exceed total fees input
    function invariant_feesNeverExceedInput() public view {
        assertLe(
            handler.ghost_totalFeesCollected(),
            handler.ghost_totalFeesIn(),
            "Collected fees exceed input"
        );
    }

    /*//////////////////////////////////////////////////////////////
                INVARIANT F2: CONSERVATION OF VALUE
    //////////////////////////////////////////////////////////////*/

    /// @notice fees_in = fees_collected + refunds (for same token)
    function invariant_feeConservation() public view {
        uint256 totalIn = handler.ghost_totalFeesIn();
        uint256 collected = handler.ghost_totalFeesCollected();
        uint256 refunds = handler.ghost_totalRefunds();

        // For same-token scenario: in = collected + refunds
        assertEq(totalIn, collected + refunds, "Fee conservation violated");
    }

    /*//////////////////////////////////////////////////////////////
                INVARIANT F3: DISTRIBUTED <= COLLECTED
    //////////////////////////////////////////////////////////////*/

    /// @notice Cannot distribute more fees than collected
    function invariant_distributionBounded() public view {
        assertLe(
            handler.ghost_totalFeesDistributed(),
            handler.ghost_totalFeesCollected(),
            "Distributed more than collected"
        );
    }

    /*//////////////////////////////////////////////////////////////
                INVARIANT F4: COLLECTED FEES CLEARED ON DISTRIBUTE
    //////////////////////////////////////////////////////////////*/

    /// @notice After distributeFees, collectedFees[validator][token] becomes 0
    /// @dev This is verified by checking that distributed == sum of all individual distributions
    function invariant_collectedFeesClearedOnDistribute() public view {
        // This invariant is implicitly checked by the handler's ghost state tracking
        // When distributeFees is called, we zero out the ghost_validatorFees mapping
        // and add to ghost_totalFeesDistributed

        // The key invariant is: undistributed fees = collected - distributed
        uint256 undistributed =
            handler.ghost_totalFeesCollected() - handler.ghost_totalFeesDistributed();

        // Undistributed fees should equal sum of all validator ghost fees
        uint256 sumValidatorFees = 0;
        for (uint256 i = 0; i < handler.validatorCount(); i++) {
            address validator = handler.getValidator(i);
            sumValidatorFees += handler.getValidatorGhostFees(validator);
        }

        assertEq(undistributed, sumValidatorFees, "Undistributed fees mismatch");
    }

    /*//////////////////////////////////////////////////////////////
                INVARIANT F5: NON-ZERO FEE ACCUMULATION
    //////////////////////////////////////////////////////////////*/

    /// @notice Fees can only be collected when actualUsed > 0
    /// @dev The handler ensures this by only adding to collected when actualUsed > 0
    function invariant_nonZeroFeeAccumulation() public view {
        // If no fees have been collected, the ghost state should reflect this
        if (handler.sameTokenFeeCalls() == 0) {
            assertEq(handler.ghost_totalFeesCollected(), 0, "Fees collected without any fee calls");
        }
    }

    /*//////////////////////////////////////////////////////////////
                        CALL SUMMARY
    //////////////////////////////////////////////////////////////*/

    /// @notice Log call statistics for debugging
    function invariant_callSummary() public view {
        console.log("=== FeeManager Invariant Call Summary ===");
        console.log("Same token fee calls:", handler.sameTokenFeeCalls());
        console.log("Distribute fee calls:", handler.distributeFeeCalls());
        console.log("Total fees in:", handler.ghost_totalFeesIn());
        console.log("Total fees collected:", handler.ghost_totalFeesCollected());
        console.log("Total refunds:", handler.ghost_totalRefunds());
        console.log("Total fees distributed:", handler.ghost_totalFeesDistributed());
    }

}
