# DEX Minimum Order Enforcement on Partial Fills

This document specifies a protocol change to prevent DoS attacks on the Stablecoin DEX by enforcing minimum order size after partial fills.

- **Spec ID**: TIP-DEX-MIN-ORDER
- **Authors/Owners**: @georgios, @dan
- **Status**: Draft
- **Related Specs**: Stablecoin DEX Specification

---

# Overview

## Abstract

When a partial fill on the Stablecoin DEX leaves an order with remaining amount below `MIN_ORDER_AMOUNT` ($100), the order is automatically cancelled and remaining tokens are refunded to the maker. This prevents DoS attacks where malicious users create arbitrarily small orders by self-matching.

## Motivation

### Problem

The current DEX enforces a $100 minimum order size at placement time, but not after partial fills. This creates a vulnerability:

1. User places a $100+ order (e.g., $150)
2. User trades against their own order to partially fill it (e.g., buy $60)
3. Order now has $90 remaining, below the minimum
4. Repeat to create arbitrarily small orders (e.g., $0.000001)

By stacking many tiny orders on the orderbook, an attacker can:
- Increase gas costs for legitimate swaps (more orders to iterate)
- Bloat storage with dust orders
- Degrade orderbook performance

### Solution

Extend the minimum order size enforcement to partial fills. When a partial fill would leave `remaining < MIN_ORDER_AMOUNT`, the order is automatically cancelled with the remaining amount refunded to the maker's internal balance.

---

# Specification

## Constants

No new constants. Uses existing:

```solidity
uint128 public constant MIN_ORDER_AMOUNT = 100_000_000; // $100 with 6 decimals
```

## Behavior Change

### Current Behavior

`partial_fill_order` updates `order.remaining` and leaves the order active regardless of the new remaining amount.

### New Behavior

After computing `new_remaining = order.remaining - fill_amount`:

1. If `new_remaining >= MIN_ORDER_AMOUNT` OR `new_remaining == 0`:
   - Continue with normal partial fill logic (no change)

2. If `0 < new_remaining < MIN_ORDER_AMOUNT`:
   - Credit the filled amount to maker (normal settlement)
   - Refund remaining tokens to maker's internal balance:
     - Bid orders: refund quote tokens (using `RoundingDirection::Up` to match escrow)
     - Ask orders: refund base tokens
   - Remove order from orderbook linked list
   - Update tick level liquidity (subtract full `order.remaining`, not just `fill_amount`)
   - If tick becomes empty, clear bitmap bit and update best tick
   - Delete order from storage
   - Emit `OrderFilled` event (with `partialFill: true`)
   - Emit `OrderCancelled` event

## Interface Changes

No interface changes. Existing events are reused:

```solidity
event OrderFilled(
    uint128 indexed orderId,
    address indexed maker,
    address indexed taker,
    uint128 fillAmount,
    bool partialFill
);

event OrderCancelled(uint128 indexed orderId);
```

When auto-cancellation occurs, both events are emitted in sequence.

## Affected Functions

- `partial_fill_order` (internal) - Primary change location
- `fill_orders_exact_in` - Calls `partial_fill_order`
- `fill_orders_exact_out` - Calls `partial_fill_order`

## Pseudocode

```rust
fn partial_fill_order(&mut self, order: &mut Order, level: &mut TickLevel, fill_amount: u128, taker: Address) -> Result<u128> {
    let new_remaining = order.remaining() - fill_amount;
    
    // Normal maker settlement for filled portion
    settle_maker(order, fill_amount);
    
    if new_remaining > 0 && new_remaining < MIN_ORDER_AMOUNT {
        // Auto-cancel: refund remaining to maker
        refund_remaining_to_maker(order, new_remaining);
        
        // Remove from orderbook
        remove_from_linked_list(order, level);
        update_tick_level_liquidity(level, order.remaining()); // Full remaining
        
        if level.head == 0 {
            clear_tick_bitmap(order);
            update_best_tick_if_needed(order);
        }
        
        delete_order(order);
        
        emit_order_filled(order, fill_amount, partial_fill: true);
        emit_order_cancelled(order);
    } else {
        // Normal partial fill
        order.remaining = new_remaining;
        update_tick_level_liquidity(level, fill_amount);
        emit_order_filled(order, fill_amount, partial_fill: true);
    }
    
    Ok(amount_out)
}
```

---

# Invariants

1. **No orders below minimum**: After any swap, no active order has `0 < remaining < MIN_ORDER_AMOUNT`

2. **Maker made whole**: When auto-cancelled, maker receives:
   - Settlement for filled portion (normal)
   - Full refund of remaining escrowed tokens

3. **Accounting consistency**: Total liquidity at tick level equals sum of remaining amounts of all orders at that tick

4. **Event ordering**: `OrderFilled` always emitted before `OrderCancelled` for auto-cancellations

## Test Cases

1. **Auto-cancel triggers**: Place $150 order, swap $60 → order cancelled, $90 refunded
2. **Boundary - at minimum**: Place $200 order, swap $100 → order remains with $100
3. **Boundary - just below**: Place $199 order, swap $100 → order cancelled, $99 refunded
4. **Full fill unaffected**: Place $100 order, swap $100 → normal full fill, no cancellation
5. **Bid order refund**: Verify quote tokens refunded with correct rounding
6. **Ask order refund**: Verify base tokens refunded exactly
7. **Linked list integrity**: Multiple orders at tick, middle order auto-cancelled
8. **Best tick updates**: Auto-cancel last order at best tick

---

# Migration

This change requires a **hard fork** as it modifies consensus-critical behavior:

- Existing orders below minimum (if any exist from edge cases) will be cancelled on next interaction
- No state migration needed - change is forward-only
- Clients should handle receiving `OrderCancelled` events for orders they didn't explicitly cancel
