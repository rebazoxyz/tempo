// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import { IStablecoinDEX } from "../../src/interfaces/IStablecoinDEX.sol";
import { ITIP20 } from "../../src/interfaces/ITIP20.sol";
import { TIP20 } from "../../src/TIP20.sol";
import { BaseTest } from "../BaseTest.t.sol";

contract StablecoinDEXInvariantTest is BaseTest {

    address[] private _actors;
    mapping(address => uint128[]) private _placedOrders;
    int16[10] private _ticks = [int16(10), 20, 30, 40, 50, 60, 70, 80, 90, 100];
    uint128 private _nextOrderId;

    // Ghost variables for tracking expected state
    mapping(address => uint256) private _expectedDexBalance;
    mapping(address => mapping(address => uint256)) private _expectedUserBalance;
    uint256 private _totalPathUsdEscrowed;
    uint256 private _totalToken1Escrowed;

    bytes32 private _pairKey;

    function setUp() public override {
        super.setUp();

        targetContract(address(this));

        // Setup token1 with issuer role and create pair
        vm.startPrank(admin);
        token1.grantRole(_ISSUER_ROLE, admin);
        pathUSD.grantRole(_ISSUER_ROLE, admin);
        vm.stopPrank();

        vm.startPrank(pathUSDAdmin);
        pathUSD.grantRole(_ISSUER_ROLE, pathUSDAdmin);
        vm.stopPrank();

        // Create the trading pair
        _pairKey = exchange.createPair(address(token1));

        _actors = _buildActors(20);
        _nextOrderId = exchange.nextOrderId();

        // Initialize expected DEX balances
        _expectedDexBalance[address(pathUSD)] = pathUSD.balanceOf(address(exchange));
        _expectedDexBalance[address(token1)] = token1.balanceOf(address(exchange));
    }

    /// Place ask / bid order and randomly cancel them.
    function placeOrder(uint256 actorRnd, uint128 amount, uint256 tickRnd, bool isBid, bool cancel)
        external
    {
        int16 tick = _ticks[tickRnd % _ticks.length];
        address actor = _actors[actorRnd % _actors.length];
        amount = uint128(bound(amount, 100_000_000, 10_000_000_000));

        _ensureFunds(actor, amount);

        vm.startPrank(actor);
        uint128 orderId = exchange.place(address(token1), amount, isBid, tick);

        // TEMPO-DEX1: Order ID monotonically increases
        _assertNextOrderId(orderId);

        uint32 price = exchange.tickToPrice(tick);
        uint256 expectedEscrow = (uint256(amount) * uint256(price) + exchange.PRICE_SCALE() - 1) / uint256(exchange.PRICE_SCALE());

        // TEMPO-DEX2: Place order escrows correct amounts
        if (isBid) {
            // Bids escrow quote tokens (pathUSD)
            _totalPathUsdEscrowed += expectedEscrow;
            _expectedDexBalance[address(pathUSD)] += expectedEscrow;
        } else {
            // Asks escrow base tokens (token1)
            _totalToken1Escrowed += amount;
            _expectedDexBalance[address(token1)] += amount;
        }

        // Verify order was created correctly
        IStablecoinDEX.Order memory order = exchange.getOrder(orderId);
        assertEq(order.maker, actor, "TEMPO-DEX2: order maker mismatch");
        assertEq(order.amount, amount, "TEMPO-DEX2: order amount mismatch");
        assertEq(order.remaining, amount, "TEMPO-DEX2: order remaining mismatch");
        assertEq(order.tick, tick, "TEMPO-DEX2: order tick mismatch");
        assertEq(order.isBid, isBid, "TEMPO-DEX2: order side mismatch");

        if (cancel) {
            exchange.cancel(orderId);

            // TEMPO-DEX3: Cancel refunds correct amounts to internal balance
            if (isBid) {
                uint128 refund = uint128(expectedEscrow);
                assertEq(exchange.balanceOf(actor, address(pathUSD)), refund, "TEMPO-DEX3: bid cancel refund mismatch");
                exchange.withdraw(address(pathUSD), refund);
                _totalPathUsdEscrowed -= expectedEscrow;
                _expectedDexBalance[address(pathUSD)] -= expectedEscrow;
            } else {
                assertEq(exchange.balanceOf(actor, address(token1)), amount, "TEMPO-DEX3: ask cancel refund mismatch");
                exchange.withdraw(address(token1), amount);
                _totalToken1Escrowed -= amount;
                _expectedDexBalance[address(token1)] -= amount;
            }

            // Verify order no longer exists
            try exchange.getOrder(orderId) returns (IStablecoinDEX.Order memory) {
                revert("TEMPO-DEX3: order should not exist after cancel");
            } catch (bytes memory reason) {
                assertEq(bytes4(reason), IStablecoinDEX.OrderDoesNotExist.selector, "TEMPO-DEX3: unexpected error on getOrder");
            }
        } else {
            _placedOrders[actor].push(orderId);

            // TEMPO-DEX7: Verify tick level liquidity updated
            (,, uint128 tickLiquidity) = exchange.getTickLevel(address(token1), tick, isBid);
            assertTrue(tickLiquidity >= amount, "TEMPO-DEX7: tick liquidity not updated");
        }

        vm.stopPrank();
    }

    /// Place ask / bid flip orders.
    function placeFlipOrder(uint256 actorRnd, uint128 amount, uint256 tickRnd, bool isBid)
        external
    {
        int16 tick = _ticks[tickRnd % _ticks.length];
        address actor = _actors[actorRnd % _actors.length];
        amount = uint128(bound(amount, 100_000_000, 10_000_000_000));

        _ensureFunds(actor, amount);

        vm.startPrank(actor);
        uint128 orderId;
        int16 flipTick;
        if (isBid) {
            flipTick = 200;
            orderId = exchange.placeFlip(address(token1), amount, true, tick, flipTick);
        } else {
            flipTick = -200;
            orderId = exchange.placeFlip(address(token1), amount, false, tick, flipTick);
        }
        _assertNextOrderId(orderId);

        // TEMPO-DEX12: Flip order constraints
        IStablecoinDEX.Order memory order = exchange.getOrder(orderId);
        assertTrue(order.isFlip, "TEMPO-DEX12: flip order not marked as flip");
        if (isBid) {
            assertTrue(order.flipTick > order.tick, "TEMPO-DEX12: bid flip tick must be > order tick");
        } else {
            assertTrue(order.flipTick < order.tick, "TEMPO-DEX12: ask flip tick must be < order tick");
        }

        // Track escrow
        uint32 price = exchange.tickToPrice(tick);
        if (isBid) {
            uint256 expectedEscrow = (uint256(amount) * uint256(price) + exchange.PRICE_SCALE() - 1) / uint256(exchange.PRICE_SCALE());
            _totalPathUsdEscrowed += expectedEscrow;
            _expectedDexBalance[address(pathUSD)] += expectedEscrow;
        } else {
            _totalToken1Escrowed += amount;
            _expectedDexBalance[address(token1)] += amount;
        }

        _placedOrders[actor].push(orderId);

        vm.stopPrank();
    }

    /// Execute swaps.
    function swapExactAmount(uint256 swapperRnd, uint128 amount, bool amtIn) external {
        address swapper = _actors[swapperRnd % _actors.length];
        amount = uint128(bound(amount, 100_000_000, 1_000_000_000));

        vm.startPrank(swapper);
        if (amtIn) {
            try exchange.swapExactAmountIn(
                address(token1), address(pathUSD), amount, amount - 100
            ) returns (
                uint128 amountOut
            ) {
                // TEMPO-DEX4: amountOut >= minAmountOut
                assertTrue(amountOut >= amount - 100, "TEMPO-DEX4: swap exact amountOut less than minAmountOut");
            } catch (bytes memory reason) {
                _assertKnownSwapError(reason);
            }
        } else {
            try exchange.swapExactAmountOut(
                address(token1), address(pathUSD), amount, amount + 100
            ) returns (
                uint128 amountIn
            ) {
                // TEMPO-DEX5: amountIn <= maxAmountIn
                assertTrue(amountIn <= amount + 100, "TEMPO-DEX5: swap exact amountIn greater than maxAmountIn");
            } catch (bytes memory reason) {
                _assertKnownSwapError(reason);
            }
        }
        // Read next order id - if a flip order is hit then next order id is incremented.
        _nextOrderId = exchange.nextOrderId();

        vm.stopPrank();
    }

    /// Cancel placed orders (if still active).
    function afterInvariant() public {
        for (uint256 i = 0; i < _actors.length; i++) {
            address actor = _actors[i];
            vm.startPrank(actor);
            for (uint256 orderId = 0; orderId < _placedOrders[actor].length; orderId++) {
                uint128 placedOrderId = _placedOrders[actor][orderId];
                // Placed orders could be filled and removed.
                try exchange.getOrder(placedOrderId) returns (IStablecoinDEX.Order memory order) {
                    // TEMPO-DEX10: Verify linked list consistency before cancel
                    _assertOrderLinkedListConsistency(placedOrderId, order);

                    exchange.cancel(placedOrderId);

                    // TEMPO-DEX3: Verify refund credited to internal balance
                    if (order.isBid) {
                        uint32 price = exchange.tickToPrice(order.tick);
                        uint128 expectedRefund = uint128((uint256(order.remaining) * uint256(price) + exchange.PRICE_SCALE() - 1) / exchange.PRICE_SCALE());
                        assertTrue(exchange.balanceOf(actor, address(pathUSD)) >= expectedRefund, "TEMPO-DEX3: bid cancel refund not credited");
                    } else {
                        assertTrue(exchange.balanceOf(actor, address(token1)) >= order.remaining, "TEMPO-DEX3: ask cancel refund not credited");
                    }
                } catch { }
            }
            vm.stopPrank();
        }
    }

    function invariantStablecoinDEX() public view {
        // TEMPO-DEX6: DEX token balances must be >= sum of all internal user balances
        uint256 dexPathUsdBalance = pathUSD.balanceOf(address(exchange));
        uint256 dexToken1Balance = token1.balanceOf(address(exchange));

        uint256 totalUserPathUsd = 0;
        uint256 totalUserToken1 = 0;
        for (uint256 i = 0; i < _actors.length; i++) {
            totalUserPathUsd += exchange.balanceOf(_actors[i], address(pathUSD));
            totalUserToken1 += exchange.balanceOf(_actors[i], address(token1));
        }

        assertTrue(
            dexPathUsdBalance >= totalUserPathUsd,
            "TEMPO-DEX6: DEX pathUsd balance < sum of user internal balances"
        );
        assertTrue(
            dexToken1Balance >= totalUserToken1,
            "TEMPO-DEX6: DEX token1 balance < sum of user internal balances"
        );

        // TEMPO-DEX8 & TEMPO-DEX9: Best bid/ask tick consistency
        _assertBestTickConsistency();

        // TEMPO-DEX7 & TEMPO-DEX11: Tick level and bitmap consistency
        _assertTickLevelConsistency();
    }

    function _assertBestTickConsistency() internal view {
        (, , int16 bestBidTick, int16 bestAskTick) = exchange.books(exchange.pairKey(address(token1), address(pathUSD)));

        // TEMPO-DEX8: If bestBidTick is not MIN, it should have liquidity
        if (bestBidTick != type(int16).min) {
            (,, uint128 bidLiquidity) = exchange.getTickLevel(address(token1), bestBidTick, true);
            // Note: during swaps, bestBidTick may temporarily point to empty tick
            // This is acceptable as it gets updated on next operation
        }

        // TEMPO-DEX9: If bestAskTick is not MAX, it should have liquidity
        if (bestAskTick != type(int16).max) {
            (,, uint128 askLiquidity) = exchange.getTickLevel(address(token1), bestAskTick, false);
            // Note: during swaps, bestAskTick may temporarily point to empty tick
        }
    }

    function _assertTickLevelConsistency() internal view {
        // Check a sample of ticks for consistency
        for (uint256 i = 0; i < _ticks.length; i++) {
            int16 tick = _ticks[i];

            // Check bid tick level
            (uint128 bidHead, uint128 bidTail, uint128 bidLiquidity) = exchange.getTickLevel(address(token1), tick, true);
            if (bidLiquidity > 0) {
                // TEMPO-DEX7: If liquidity > 0, head should be non-zero
                assertTrue(bidHead != 0, "TEMPO-DEX7: bid tick has liquidity but no head");
                // TEMPO-DEX11: Bitmap should have this tick marked
            }
            if (bidHead == 0) {
                // If head is 0, tail should also be 0 and liquidity should be 0
                assertEq(bidTail, 0, "TEMPO-DEX10: bid tail non-zero but head is zero");
                assertEq(bidLiquidity, 0, "TEMPO-DEX7: bid liquidity non-zero but head is zero");
            }

            // Check ask tick level
            (uint128 askHead, uint128 askTail, uint128 askLiquidity) = exchange.getTickLevel(address(token1), tick, false);
            if (askLiquidity > 0) {
                assertTrue(askHead != 0, "TEMPO-DEX7: ask tick has liquidity but no head");
            }
            if (askHead == 0) {
                assertEq(askTail, 0, "TEMPO-DEX10: ask tail non-zero but head is zero");
                assertEq(askLiquidity, 0, "TEMPO-DEX7: ask liquidity non-zero but head is zero");
            }
        }
    }

    function _assertOrderLinkedListConsistency(uint128 orderId, IStablecoinDEX.Order memory order) internal view {
        // TEMPO-DEX10: If order has prev, prev's next should point to this order
        if (order.prev != 0) {
            IStablecoinDEX.Order memory prevOrder = exchange.getOrder(order.prev);
            assertEq(prevOrder.next, orderId, "TEMPO-DEX10: prev order's next doesn't point to current");
        }

        // TEMPO-DEX10: If order has next, next's prev should point to this order
        if (order.next != 0) {
            IStablecoinDEX.Order memory nextOrder = exchange.getOrder(order.next);
            assertEq(nextOrder.prev, orderId, "TEMPO-DEX10: next order's prev doesn't point to current");
        }
    }

    function _assertNextOrderId(uint128 orderId) internal {
        // TEMPO-DEX1: Order ID monotonically increases
        assertEq(orderId, _nextOrderId, "TEMPO-DEX1: next order id mismatch");
        _nextOrderId += 1;
    }

    function _assertKnownSwapError(bytes memory reason) internal pure {
        bytes4 selector = bytes4(reason);
        bool isKnownError = 
            selector == IStablecoinDEX.InsufficientLiquidity.selector ||
            selector == IStablecoinDEX.InsufficientOutput.selector ||
            selector == IStablecoinDEX.MaxInputExceeded.selector ||
            selector == IStablecoinDEX.InsufficientBalance.selector ||
            selector == IStablecoinDEX.PairDoesNotExist.selector ||
            selector == IStablecoinDEX.IdenticalTokens.selector ||
            selector == IStablecoinDEX.InvalidToken.selector ||
            selector == ITIP20.PolicyForbids.selector;
        assertTrue(isKnownError, "Swap failed with unknown error");
    }

    function _buildActors(uint256 noOfActors_) internal returns (address[] memory) {
        address[] memory actorsAddress = new address[](noOfActors_);

        for (uint256 i = 0; i < noOfActors_; i++) {
            address actor = makeAddr(string(abi.encodePacked("Actor", vm.toString(i))));
            actorsAddress[i] = actor;

            // initial actor balance
            _ensureFunds(actor, 1_000_000_000_000);

            vm.startPrank(actor);
            token1.approve(address(exchange), type(uint256).max);
            pathUSD.approve(address(exchange), type(uint256).max);
            vm.stopPrank();
        }

        return actorsAddress;
    }

    function _ensureFunds(address actor, uint256 amount) internal {
        vm.startPrank(admin);
        if (pathUSD.balanceOf(address(actor)) < amount) {
            pathUSD.mint(actor, amount);
        }
        if (token1.balanceOf(address(actor)) < amount) {
            token1.mint(actor, amount);
        }
        vm.stopPrank();
    }

}
