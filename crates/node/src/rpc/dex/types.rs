use alloy::primitives::{Address, B256};
use serde::{Deserialize, Serialize, Serializer, ser::SerializeStruct};

pub type OrdersParams = PaginationParams<OrdersFilters>;
pub type Tick = i16;

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct PaginationResponse<Item> {
    /// Cursor for next page, null if no more results
    pub next_cursor: Option<String>,
    /// Array of items matching the input query
    pub items: Vec<Item>,
}

/// A trait whose implementation determines a field name for [`PaginationResponse`] implementation
/// of [`Serialize`]r.
pub trait FieldName {
    /// Returns the camel case plural name for this field.
    ///
    /// For example a struct called `RoleChange` would likely return `"roleChanges"`.
    fn field_plural_camel_case() -> &'static str;
}

impl<Item: Serialize + FieldName> Serialize for PaginationResponse<Item> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut ser = serializer.serialize_struct("PaginationResponse", 2)?;
        ser.serialize_field("nextCursor", &self.next_cursor)?;
        ser.serialize_field(Item::field_plural_camel_case(), &self.items)?;
        ser.end()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaginationParams<Filters> {
    /// Cursor for pagination. Based on orderId.
    ///
    /// Defaults to first entry based on the sort and filter configuration.
    /// Use the `nextCursor` in response to get the next set of orders.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,

    /// Determines which items should be yielded in the response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filters: Option<Filters>,

    /// Maximum number of orders to return.
    ///
    /// Defaults to 10.
    /// Maximum is 100.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,

    /// Determines the order of the items yielded in the response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<OrdersSort>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrdersSort {
    /// A field the items are compared with.
    pub on: String,

    /// An ordering direction.
    pub order: OrdersSortOrder,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OrdersSortOrder {
    Asc,
    #[default]
    Desc,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrdersFilters {
    /// Filter by specific base token
    pub base_token: Option<Address>,
    /// Filter by order side (true=buy, false=sell)
    pub is_bid: Option<bool>,
    /// Filter flip orders
    pub is_flip: Option<bool>,
    /// Filter by maker address
    pub maker: Option<Address>,
    /// Filter by quote token
    pub quote_token: Option<Address>,
    /// Remaining amount in range
    pub remaining: Option<FilterRange<u128>>,
    /// Tick in range (from -2000 to 2000)
    pub tick: Option<FilterRange<Tick>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterRange<T> {
    pub min: Option<T>,
    pub max: Option<T>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Order {
    /// Original order amount
    pub amount: B256,
    /// Target tick to flip to when order is filled
    pub flip_tick: i16,
    /// Order side: true for buy (bid), false for sell (ask)
    pub is_bid: bool,
    /// Whether this is a flip order that auto-flips when filled
    pub is_flip: bool,
    /// Address of order maker
    pub maker: Address,
    /// Next order ID in FIFO queue
    #[serde(with = "alloy_serde::quantity")]
    pub next: u128,
    /// Unique order ID
    #[serde(with = "alloy_serde::quantity")]
    pub order_id: u128,
    /// Previous order ID in FIFO queue
    #[serde(with = "alloy_serde::quantity")]
    pub prev: u128,
    /// Remaining amount to fill
    #[serde(with = "alloy_serde::quantity")]
    pub remaining: u128,
    /// Price tick
    pub tick: i16,
    /// Address of the base token
    pub base_token: Address,
    /// Address of the quote token
    pub quote_token: Address,
}

impl FieldName for Order {
    fn field_plural_camel_case() -> &'static str {
        "orders"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case::test_case(
        OrdersParams::default();
        "None filled"
    )]
    fn test_serialize_and_deserialize_is_identical(expected_params: OrdersParams) {
        let json = serde_json::to_string(&expected_params).unwrap();
        let actual_params: OrdersParams = serde_json::from_str(&json).unwrap();

        assert_eq!(actual_params, expected_params);
    }

    #[test_case::test_case(
        "{}";
        "None filled"
    )]
    fn test_deserialize_and_serialize_is_identical(expected_json: &str) {
        let params: OrdersParams = serde_json::from_str(expected_json).unwrap();
        let actual_json = serde_json::to_string(&params).unwrap();

        assert_eq!(actual_json, expected_json);
    }
}
