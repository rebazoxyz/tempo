//! Event encoding parity tests between `#[solidity]` and `sol!`.

use alloy::primitives::{Address, U256};
use alloy::sol_types::SolEvent;
use alloy_sol_macro::sol;
use tempo_precompiles_macros::solidity;

use super::assert_event_parity;

sol! {
    #[derive(Debug, PartialEq, Eq)]
    event Transfer(address indexed from, address indexed to, uint256 amount);

    #[derive(Debug, PartialEq, Eq)]
    event Log(string message, uint256 value);

    #[derive(Debug, PartialEq, Eq)]
    event TripleIndexed(address indexed a, address indexed b, uint256 indexed c);

    #[derive(Debug, PartialEq, Eq)]
    event Approval(address indexed owner, address indexed spender, uint256 value);
}

#[solidity]
mod rust {
    use super::*;

    pub enum Event {
        Transfer {
            #[indexed]
            from: Address,
            #[indexed]
            to: Address,
            amount: U256,
        },
        Log {
            message: String,
            value: U256,
        },
        TripleIndexed {
            #[indexed]
            a: Address,
            #[indexed]
            b: Address,
            #[indexed]
            c: U256,
        },
        Approval {
            #[indexed]
            owner: Address,
            #[indexed]
            spender: Address,
            value: U256,
        },
    }
}

#[test]
fn indexed_event_parity() {
    assert_event_parity::<Transfer, rust::Transfer>();
}

#[test]
fn indexed_event_topics() {
    let sol_event = Transfer {
        from: Address::repeat_byte(0x11),
        to: Address::repeat_byte(0x22),
        amount: U256::from(1000),
    };
    let our_event = rust::Transfer {
        from: Address::repeat_byte(0x11),
        to: Address::repeat_byte(0x22),
        amount: U256::from(1000),
    };

    // Compare topic encoding
    let sol_topics = sol_event.encode_topics_array::<3>();
    let our_topics = our_event.encode_topics_array::<3>();
    assert_eq!(sol_topics, our_topics, "Event topics mismatch");

    // Compare data encoding
    let sol_data = sol_event.encode_data();
    let our_data = our_event.encode_data();
    assert_eq!(sol_data, our_data, "Event data mismatch");
}

#[test]
fn data_only_event_parity() {
    assert_event_parity::<Log, rust::Log>();
}

#[test]
fn data_only_event_encoding() {
    let sol_event = Log {
        message: "Hello".to_string(),
        value: U256::from(42),
    };
    let our_event = rust::Log {
        message: "Hello".to_string(),
        value: U256::from(42),
    };

    // Only topic[0] (signature hash), rest is data
    let sol_topics = sol_event.encode_topics_array::<1>();
    let our_topics = our_event.encode_topics_array::<1>();
    assert_eq!(sol_topics, our_topics);

    let sol_data = sol_event.encode_data();
    let our_data = our_event.encode_data();
    assert_eq!(sol_data, our_data);
}

#[test]
fn triple_indexed_parity() {
    assert_event_parity::<TripleIndexed, rust::TripleIndexed>();
}

#[test]
fn triple_indexed_topics() {
    let sol_event = TripleIndexed {
        a: Address::repeat_byte(0x01),
        b: Address::repeat_byte(0x02),
        c: U256::from(3),
    };
    let our_event = rust::TripleIndexed {
        a: Address::repeat_byte(0x01),
        b: Address::repeat_byte(0x02),
        c: U256::from(3),
    };

    // 4 topics: signature + 3 indexed values
    let sol_topics = sol_event.encode_topics_array::<4>();
    let our_topics = our_event.encode_topics_array::<4>();
    assert_eq!(sol_topics, our_topics);

    // No data since all fields are indexed
    let sol_data = sol_event.encode_data();
    let our_data = our_event.encode_data();
    assert_eq!(sol_data, our_data);
    assert!(sol_data.is_empty());
}

#[test]
fn approval_event_parity() {
    assert_event_parity::<Approval, rust::Approval>();

    let sol_event = Approval {
        owner: Address::repeat_byte(0xAA),
        spender: Address::repeat_byte(0xBB),
        value: U256::MAX,
    };
    let our_event = rust::Approval {
        owner: Address::repeat_byte(0xAA),
        spender: Address::repeat_byte(0xBB),
        value: U256::MAX,
    };

    // 3 topics: signature + owner + spender
    let sol_topics = sol_event.encode_topics_array::<3>();
    let our_topics = our_event.encode_topics_array::<3>();
    assert_eq!(sol_topics, our_topics);

    // Data: value
    let sol_data = sol_event.encode_data();
    let our_data = our_event.encode_data();
    assert_eq!(sol_data, our_data);
}
