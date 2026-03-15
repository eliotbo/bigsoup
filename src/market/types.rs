#[derive(Clone, Copy, Debug)]
pub struct BBO {
    pub best_bid: f32,
    pub best_bid_size: f32,
    pub best_ask: f32,
    pub best_ask_size: f32,
    pub last_price: f32,
    pub tick: u64,
    /// Exogenous fundamental value injected by the simulation each tick.
    /// Defaults to `last_price` when no exogenous process is configured.
    pub fair_value: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Order {
    pub agent_id: u32,
    pub price: f32,
    pub quantity: f32, // positive = buy, negative = sell
}

#[derive(Clone, Copy, Debug)]
pub struct Trade {
    pub buyer_id: u32,
    pub seller_id: u32,
    pub price: f32,
    pub quantity: f32,
    pub tick: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderType {
    Limit,
    Market,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Buy,
    Sell,
}

#[derive(Clone, Debug)]
pub struct LobOrder {
    pub order_id: u64,
    pub agent_id: u32,
    pub side: Side,
    pub price: f32,
    pub quantity: f32,
    pub order_type: OrderType,
    pub tick: u64,
}
