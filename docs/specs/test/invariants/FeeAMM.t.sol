// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import { FeeAMM } from "../../src/FeeAMM.sol";
import { TIP20 } from "../../src/TIP20.sol";
import { TIP20Factory } from "../../src/TIP20Factory.sol";
import { IFeeAMM } from "../../src/interfaces/IFeeAMM.sol";
import { ITIP20 } from "../../src/interfaces/ITIP20.sol";
import { Test } from "forge-std/Test.sol";

contract FeeAMMSingleInvariantTest is Test {

    ITIP20 pathUsd = ITIP20(0x20C0000000000000000000000000000000000000);
    ITIP20 alphaUsd = ITIP20(0x20C0000000000000000000000000000000000001);
    ITIP20 betaUsd = ITIP20(0x20C0000000000000000000000000000000000002);
    ITIP20 thetaUsd = ITIP20(0x20C0000000000000000000000000000000000003);

    FeeAMM internal constant FEE_AMM = FeeAMM(0xfeEC000000000000000000000000000000000000);
    TIP20Factory internal constant TIP20_FACTORY =
        TIP20Factory(0x20Fc000000000000000000000000000000000000);

    TIP20[] public usdTokens;
    address[] public actors;

    bytes32[] public poolIds;
    mapping(bytes32 => bool) public seenPool;

    function setUp() public {
        usdTokens.push(TIP20(address(pathUsd)));
        usdTokens.push(TIP20(address(alphaUsd)));
        usdTokens.push(TIP20(address(betaUsd)));
        usdTokens.push(TIP20(address(thetaUsd)));

        for (uint256 i = 0; i < 20; i++) {
            actors.push(makeAddr(string(abi.encodePacked("actor-", vm.toString(i)))));
        }

        for (uint256 i = 0; i < actors.length; i++) {
            address a = actors[i];
            for (uint256 j = 0; j < usdTokens.length; j++) {
                usdTokens[j].mintWithMemo(a, 5_000_000e18, bytes32(0));
                vm.prank(a);
                usdTokens[j].approve(address(FEE_AMM), type(uint256).max);
            }
        }

        targetContract(address(this));

        bytes4[] memory selectors = new bytes4[](3);
        selectors[0] = this.act_mint.selector;
        selectors[1] = this.act_burn.selector;
        selectors[2] = this.act_rebalanceSwap.selector;
        targetSelector(FuzzSelector({ addr: address(this), selectors: selectors }));
    }

    /*//////////////////////////////////////////////////////////////
                               ACTIONS
    //////////////////////////////////////////////////////////////*/

    function act_mint(uint256 seedA, uint256 seedB, uint256 seedActor, uint256 rawAmount) external {
        (TIP20 userToken, TIP20 validatorToken) = _pair(seedA, seedB);
        address actor = _actor(seedActor);

        uint256 amount = bound(rawAmount, 0, 1_000_000e18);

        vm.startPrank(actor);
        try FEE_AMM.mint(address(userToken), address(validatorToken), amount, actor) returns (
            uint256
        ) {
            _rememberPool(address(userToken), address(validatorToken));
        } catch { }
        vm.stopPrank();
    }

    function act_burn(uint256 seedA, uint256 seedB, uint256 seedActor, uint256 rawLiq) external {
        (TIP20 userToken, TIP20 validatorToken) = _pair(seedA, seedB);
        address actor = _actor(seedActor);

        bytes32 pid = FEE_AMM.getPoolId(address(userToken), address(validatorToken));
        uint256 bal = FEE_AMM.liquidityBalances(pid, actor);

        uint256 liq = (bal == 0) ? bound(rawLiq, 0, 1e18) : bound(rawLiq, 0, bal);

        vm.startPrank(actor);
        try FEE_AMM.burn(address(userToken), address(validatorToken), liq, actor) returns (
            uint256, uint256
        ) {
            _rememberPool(address(userToken), address(validatorToken));
        } catch { }
        vm.stopPrank();
    }

    function act_rebalanceSwap(uint256 seedA, uint256 seedB, uint256 seedActor, uint256 rawOut)
        external
    {
        (TIP20 userToken, TIP20 validatorToken) = _pair(seedA, seedB);
        address actor = _actor(seedActor);

        // Snapshot state for step checks.
        IFeeAMM.Pool memory beforeP = FEE_AMM.getPool(address(userToken), address(validatorToken));

        // If no user-token liquidity exists, don't attempt (avoids meaningless amountOut=0 donation path).
        uint256 maxOut = uint256(beforeP.reserveUserToken);
        if (maxOut == 0) return;

        // Force amountOut to be strictly positive to avoid amountIn rounding to +1 when outAmt=0.
        uint256 outAmt = bound(rawOut, 1, maxOut);

        // Expected amountIn = floor(outAmt * N / SCALE) + 1, with N=9985, SCALE=10000.
        uint256 expectedIn = (outAmt * 9985) / 10_000 + 1;

        vm.startPrank(actor);
        try FEE_AMM.rebalanceSwap(
            address(userToken), address(validatorToken), outAmt, actor
        ) returns (
            uint256 amountIn
        ) {
            _rememberPool(address(userToken), address(validatorToken));

            // Step invariant: returned amountIn matches formula.
            assertEq(amountIn, expectedIn, "rebalanceSwap amountIn mismatch");

            // Step invariant: reserves update exactly.
            IFeeAMM.Pool memory afterP =
                FEE_AMM.getPool(address(userToken), address(validatorToken));
            assertEq(
                uint256(afterP.reserveValidatorToken),
                uint256(beforeP.reserveValidatorToken) + amountIn,
                "reserveValidatorToken delta mismatch"
            );
            assertEq(
                uint256(afterP.reserveUserToken),
                uint256(beforeP.reserveUserToken) - outAmt,
                "reserveUserToken delta mismatch"
            );
        } catch { }
        vm.stopPrank();
    }

    /*//////////////////////////////////////////////////////////////
                               INVARIANTS
    //////////////////////////////////////////////////////////////*/

    // Invariant 1: Pool initialization shape
    // - A pool is either completely uninitialized, or properly initialized.
    //   * If totalSupply == 0, then both reserves must be zero.
    //   * If totalSupply > 0, then the pool must have locked at least MIN_LIQUIDITY.
    function invariant_pool_supply_and_reserve_shape() public view {
        uint256 minLiq = FEE_AMM.MIN_LIQUIDITY();

        for (uint256 i = 0; i < poolIds.length; i++) {
            bytes32 pid = poolIds[i];
            (uint128 ru, uint128 rv) = FEE_AMM.pools(pid);
            uint256 supply = FEE_AMM.totalSupply(pid);

            if (supply == 0) {
                assertEq(uint256(ru), 0, "supply=0 => reserveU=0");
                assertEq(uint256(rv), 0, "supply=0 => reserveV=0");
            } else {
                assertGe(supply, minLiq, "initialized pool must lock MIN_LIQUIDITY");
            }
        }
    }

    // Invariant 2: LP supply accounting
    // For every pool that has been touched by the fuzz actions:
    // - The total LP supply equals the sum of all actor LP balances plus the permanently locked MIN_LIQUIDITY.
    //   This ensures that:
    //     * The locked MIN_LIQUIDITY is not owned by any actor.
    //     * No actor balance can exceed the total supply.
    //     * LP accounting is conserved across mint and burn operations.
    function invariant_lp_accounting_matches_locked_min_liquidity() public view {
        uint256 minLiq = FEE_AMM.MIN_LIQUIDITY();

        for (uint256 i = 0; i < poolIds.length; i++) {
            bytes32 pid = poolIds[i];
            uint256 supply = FEE_AMM.totalSupply(pid);

            // If supply is zero, this pool is considered uninitialized by our harness.
            if (supply == 0) continue;

            uint256 sum = _sumActorLp(pid);

            // Strong accounting identity: all LP owned by actors + locked MIN_LIQUIDITY == totalSupply.
            assertEq(supply, sum + minLiq, "supply != sumBalances + MIN_LIQUIDITY");

            // Local sanity: no single actor can exceed totalSupply.
            // (Not strictly needed if above passes, but gives better shrink output.)
            for (uint256 k = 0; k < actors.length; k++) {
                uint256 bal = FEE_AMM.liquidityBalances(pid, actors[k]);
                assertLe(bal, supply, "actor LP balance > totalSupply");
            }
        }
    }

    // Invariant 3: Reserve backing by on-chain balances
    // For each USD token in the test universe:
    // - The AMM’s on-chain token balance must be at least the sum of that token’s reserves
    //   across all pools that reference it.
    //   This guarantees that pool reserves are always fully backed by actual token balances
    //   held by the AMM contract.
    function invariant_token_balance_covers_sum_of_reserves() public view {
        uint256 n = usdTokens.length;
        uint256[] memory sumReserves = new uint256[](n);

        // Accumulate reserves across all seen pools in the token universe.
        for (uint256 a = 0; a < n; a++) {
            for (uint256 b = 0; b < n; b++) {
                if (a == b) continue;

                address userToken = address(usdTokens[a]);
                address validatorToken = address(usdTokens[b]);

                bytes32 pid = FEE_AMM.getPoolId(userToken, validatorToken);
                if (!seenPool[pid]) continue;

                // Read pool reserves once.
                IFeeAMM.Pool memory p = FEE_AMM.getPool(userToken, validatorToken);

                // Reserves are uint128 in storage; assert that decoded values are in-range.
                // (Catches storage corruption/packing issues.)
                assertLe(uint256(p.reserveUserToken), type(uint128).max, "reserveUserToken > u128");
                assertLe(
                    uint256(p.reserveValidatorToken),
                    type(uint128).max,
                    "reserveValidatorToken > u128"
                );

                sumReserves[a] += uint256(p.reserveUserToken);
                sumReserves[b] += uint256(p.reserveValidatorToken);
            }
        }

        // Check AMM balances cover aggregate reserves per token.
        for (uint256 i = 0; i < n; i++) {
            uint256 bal = usdTokens[i].balanceOf(address(FEE_AMM));
            assertGe(bal, sumReserves[i], "token balance < sum(reserves)");
        }
    }

    /// Invariant 4:
    /// Every tracked poolId must correspond to exactly one ordered pair in this test's token universe.
    function invariant_pool_ids_resolve_to_unique_ordered_pair() public view {
        for (uint256 i = 0; i < poolIds.length; i++) {
            bytes32 pid = poolIds[i];
            uint256 matches;

            for (uint256 a = 0; a < usdTokens.length; a++) {
                for (uint256 b = 0; b < usdTokens.length; b++) {
                    if (a == b) continue;
                    if (FEE_AMM.getPoolId(address(usdTokens[a]), address(usdTokens[b])) == pid) {
                        matches++;
                    }
                }
            }

            assertEq(matches, 1, "poolId must match exactly one ordered pair");
        }
    }

    /*//////////////////////////////////////////////////////////////
                               HELPERS
    //////////////////////////////////////////////////////////////*/

    function _actor(uint256 seed) internal view returns (address) {
        return actors[seed % actors.length];
    }

    function _token(uint256 seed) internal view returns (TIP20) {
        return usdTokens[seed % usdTokens.length];
    }

    function _pair(uint256 seedA, uint256 seedB)
        internal
        view
        returns (TIP20 userToken, TIP20 validatorToken)
    {
        uint256 n = usdTokens.length;

        uint256 ia = seedA % n;
        uint256 ib = seedB % n;

        if (ia == ib) {
            ib = (ib + 1) % n;
        }

        userToken = usdTokens[ia];
        validatorToken = usdTokens[ib];
    }

    function _sumActorLp(bytes32 pid) internal view returns (uint256 sum) {
        for (uint256 k = 0; k < actors.length; k++) {
            sum += FEE_AMM.liquidityBalances(pid, actors[k]);
        }
    }

    function _rememberPool(address userToken, address validatorToken) internal {
        if (userToken == validatorToken) return;
        bytes32 pid = FEE_AMM.getPoolId(userToken, validatorToken);
        if (!seenPool[pid]) {
            seenPool[pid] = true;
            poolIds.push(pid);
        }
    }

}
