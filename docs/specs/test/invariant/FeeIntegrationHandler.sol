// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import { FeeManager } from "../../src/FeeManager.sol";
import { TIP20 } from "../../src/TIP20.sol";
import { IFeeAMM } from "../../src/interfaces/IFeeAMM.sol";
import { CommonBase } from "forge-std/Base.sol";
import { StdCheats } from "forge-std/StdCheats.sol";
import { StdUtils } from "forge-std/StdUtils.sol";

/// @title FeeIntegrationHandler
/// @notice Combined handler for FeeManager + FeeAMM integration invariant testing
/// @dev Tests cross-contract interactions between fee collection and AMM operations
contract FeeIntegrationHandler is CommonBase, StdCheats, StdUtils {

    FeeManager public feeManager;
    TIP20 public userToken;
    TIP20 public validatorToken;
    bytes32 public poolId;
    address public admin;

    // Constants matching FeeAMM
    uint256 public constant M = 9970; // Fee swap rate
    uint256 public constant N = 9985; // Rebalance swap rate
    uint256 public constant SCALE = 10_000;

    // Ghost state - Fee tracking
    uint256 public ghost_totalFeesIn;
    uint256 public ghost_totalFeesCollected;
    uint256 public ghost_totalRefunds;
    uint256 public ghost_totalFeesDistributed;

    // Ghost state - AMM tracking
    uint256 public ghost_totalMinted;
    uint256 public ghost_totalBurned;
    uint256 public ghost_rebalanceIn;
    uint256 public ghost_rebalanceOut;

    // Ghost state - Cross-token fee tracking
    uint256 public ghost_crossTokenFeesIn;
    uint256 public ghost_crossTokenFeesOut;

    // Track LP balances per actor
    mapping(address => uint256) public ghost_lpBalances;

    address[] public actors;

    // Call counters
    uint256 public sameTokenFeeCalls;
    uint256 public crossTokenFeeCalls;
    uint256 public mintCalls;
    uint256 public burnCalls;
    uint256 public rebalanceCalls;
    uint256 public distributeFeeCalls;

    constructor(FeeManager _feeManager, TIP20 _userToken, TIP20 _validatorToken, address _admin) {
        feeManager = _feeManager;
        userToken = _userToken;
        validatorToken = _validatorToken;
        poolId = feeManager.getPoolId(address(userToken), address(validatorToken));
        admin = _admin;

        // Setup actors
        actors.push(address(0x4001));
        actors.push(address(0x4002));
        actors.push(address(0x4003));
    }

    /// @notice Simulate same-token fee collection
    function simulateSameTokenFee(uint256 actorSeed, uint256 maxAmount, uint256 actualUsedPct)
        external
    {
        address actor = actors[actorSeed % actors.length];

        maxAmount = bound(maxAmount, 1e6, 1_000_000e18);
        actualUsedPct = bound(actualUsedPct, 0, 100);
        uint256 actualUsed = (maxAmount * actualUsedPct) / 100;
        uint256 refund = maxAmount - actualUsed;

        // Mint tokens to actor (handler has ISSUER_ROLE)
        userToken.mint(actor, maxAmount);

        vm.prank(actor);
        userToken.transfer(address(feeManager), maxAmount);
        ghost_totalFeesIn += maxAmount;

        if (refund > 0) {
            vm.prank(address(feeManager));
            userToken.transfer(actor, refund);
            ghost_totalRefunds += refund;
        }

        ghost_totalFeesCollected += actualUsed;
        sameTokenFeeCalls++;
    }

    /// @notice Simulate cross-token fee collection (userToken -> validatorToken swap)
    /// @dev This simulates what happens when user pays in userToken but validator wants validatorToken
    function simulateCrossTokenFee(uint256 actorSeed, uint256 maxAmount, uint256 actualUsedPct)
        external
    {
        address actor = actors[actorSeed % actors.length];

        maxAmount = bound(maxAmount, 1e6, 1_000_000e18);
        actualUsedPct = bound(actualUsedPct, 1, 100); // At least 1% to ensure swap happens
        uint256 actualUsed = (maxAmount * actualUsedPct) / 100;
        uint256 refund = maxAmount - actualUsed;

        // Check if pool has sufficient liquidity
        IFeeAMM.Pool memory pool = feeManager.getPool(address(userToken), address(validatorToken));
        uint256 amountOutNeeded = (actualUsed * M) / SCALE;
        if (pool.reserveValidatorToken < amountOutNeeded) return;

        // Mint tokens to actor (handler has ISSUER_ROLE)
        userToken.mint(actor, maxAmount);

        // Pre-tx: transfer max to FeeManager
        vm.prank(actor);
        userToken.transfer(address(feeManager), maxAmount);
        ghost_totalFeesIn += maxAmount;
        ghost_crossTokenFeesIn += actualUsed;

        // Post-tx: refund unused
        if (refund > 0) {
            vm.prank(address(feeManager));
            userToken.transfer(actor, refund);
            ghost_totalRefunds += refund;
        }

        // Execute swap: actualUsed userToken -> validatorToken at rate M/SCALE
        uint256 amountOut = (actualUsed * M) / SCALE;
        ghost_crossTokenFeesOut += amountOut;
        ghost_totalFeesCollected += amountOut;

        crossTokenFeeCalls++;
    }

    /// @notice Add liquidity to the pool
    function addLiquidity(uint256 actorSeed, uint256 amount) external {
        address actor = actors[actorSeed % actors.length];

        amount = bound(amount, 2002, 10_000_000e18);

        // Mint tokens to actor (handler has ISSUER_ROLE)
        validatorToken.mint(actor, amount);

        vm.startPrank(actor);
        validatorToken.approve(address(feeManager), amount);

        try feeManager.mint(address(userToken), address(validatorToken), amount, actor) returns (
            uint256 liquidity
        ) {
            ghost_totalMinted += liquidity;
            ghost_lpBalances[actor] += liquidity;
            mintCalls++;
        } catch {
            // Expected to fail sometimes
        }
        vm.stopPrank();
    }

    /// @notice Remove liquidity from the pool
    function removeLiquidity(uint256 actorSeed, uint256 pct) external {
        address actor = actors[actorSeed % actors.length];

        uint256 balance = feeManager.liquidityBalances(poolId, actor);
        if (balance == 0) return;

        pct = bound(pct, 1, 100);
        uint256 amount = (balance * pct) / 100;
        if (amount == 0) return;

        vm.startPrank(actor);
        try feeManager.burn(address(userToken), address(validatorToken), amount, actor) returns (
            uint256, uint256
        ) {
            ghost_totalBurned += amount;
            ghost_lpBalances[actor] -= amount;
            burnCalls++;
        } catch {
            // Expected to fail sometimes
        }
        vm.stopPrank();
    }

    /// @notice Rebalance swap (validatorToken -> userToken)
    function rebalancePool(uint256 actorSeed, uint256 amountOut) external {
        address actor = actors[actorSeed % actors.length];

        IFeeAMM.Pool memory pool = feeManager.getPool(address(userToken), address(validatorToken));
        if (pool.reserveUserToken == 0) return;

        amountOut = bound(amountOut, 1, pool.reserveUserToken);

        uint256 amountIn = (amountOut * N) / SCALE + 1;

        // Mint tokens to actor (handler has ISSUER_ROLE)
        validatorToken.mint(actor, amountIn);

        vm.startPrank(actor);
        validatorToken.approve(address(feeManager), amountIn);

        try feeManager.rebalanceSwap(
            address(userToken), address(validatorToken), amountOut, actor
        ) returns (
            uint256 actualIn
        ) {
            ghost_rebalanceIn += actualIn;
            ghost_rebalanceOut += amountOut;
            rebalanceCalls++;
        } catch {
            // May fail if reserves depleted
        }
        vm.stopPrank();
    }

    /// @notice Distribute accumulated fees
    function distributeFees(uint256 actorSeed) external {
        address actor = actors[actorSeed % actors.length];

        // Try distributing userToken fees
        uint256 userTokenFees = feeManager.collectedFees(actor, address(userToken));
        if (userTokenFees > 0) {
            feeManager.distributeFees(actor, address(userToken));
            ghost_totalFeesDistributed += userTokenFees;
            distributeFeeCalls++;
            return;
        }

        // Try distributing validatorToken fees
        uint256 validatorTokenFees = feeManager.collectedFees(actor, address(validatorToken));
        if (validatorTokenFees > 0) {
            feeManager.distributeFees(actor, address(validatorToken));
            ghost_totalFeesDistributed += validatorTokenFees;
            distributeFeeCalls++;
        }
    }

    // View helpers

    function sumLPBalances() external view returns (uint256 total) {
        for (uint256 i = 0; i < actors.length; i++) {
            total += ghost_lpBalances[actors[i]];
        }
    }

    function actorCount() external view returns (uint256) {
        return actors.length;
    }

    function getActor(uint256 index) external view returns (address) {
        return actors[index];
    }

}
