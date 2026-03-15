use std::collections::{BTreeMap, HashMap, VecDeque};

use ordered_float::OrderedFloat;
use smallvec::SmallVec;

use super::types::{BBO, LobOrder, OrderType, Side, Trade};

pub struct PriceLevel {
    pub orders: VecDeque<LobOrder>,
    pub total_quantity: f32,
}

pub struct BookSide {
    pub levels: BTreeMap<OrderedFloat<f32>, PriceLevel>,
}

impl BookSide {
    fn new() -> Self {
        Self {
            levels: BTreeMap::new(),
        }
    }

    fn total_orders(&self) -> usize {
        self.levels.values().map(|l| l.orders.len()).sum()
    }

    fn total_quantity(&self) -> f32 {
        self.levels.values().map(|l| l.total_quantity).sum()
    }
}

pub struct LimitOrderBook {
    bids: BookSide,
    asks: BookSide,
    next_order_id: u64,
    last_price: f32,
    tick: u64,
    agent_orders: HashMap<u32, SmallVec<[u64; 4]>>,
    order_index: HashMap<u64, (Side, OrderedFloat<f32>)>,
}

impl LimitOrderBook {
    pub fn new(initial_price: f32) -> Self {
        Self {
            bids: BookSide::new(),
            asks: BookSide::new(),
            next_order_id: 0,
            last_price: initial_price,
            tick: 0,
            agent_orders: HashMap::new(),
            order_index: HashMap::new(),
        }
    }

    /// Remove all resting orders for the given agent.
    pub fn cancel_agent(&mut self, agent_id: u32) {
        if let Some(order_ids) = self.agent_orders.remove(&agent_id) {
            for order_id in order_ids {
                if let Some((side, price)) = self.order_index.remove(&order_id) {
                    let book_side = match side {
                        Side::Buy => &mut self.bids,
                        Side::Sell => &mut self.asks,
                    };
                    let should_remove =
                        if let Some(level) = book_side.levels.get_mut(&price) {
                            level.orders.retain(|o| o.order_id != order_id);
                            level.total_quantity =
                                level.orders.iter().map(|o| o.quantity).sum();
                            level.orders.is_empty()
                        } else {
                            false
                        };
                    if should_remove {
                        book_side.levels.remove(&price);
                    }
                }
            }
        }
    }

    /// Submit an order: match against the book, then rest any remainder (limit only).
    pub fn submit_order(&mut self, mut order: LobOrder) -> Vec<Trade> {
        order.order_id = self.next_order_id;
        self.next_order_id += 1;

        let trades = match order.side {
            Side::Buy => self.match_buy_order(&mut order),
            Side::Sell => self.match_sell_order(&mut order),
        };

        if let Some(last) = trades.last() {
            self.last_price = last.price;
        }

        // Rest remaining quantity for limit orders
        if order.order_type == OrderType::Limit && order.quantity > f32::EPSILON {
            self.rest_order(order);
        }

        trades
    }

    /// Match an incoming buy against resting asks (ascending by price).
    fn match_buy_order(&mut self, order: &mut LobOrder) -> Vec<Trade> {
        let mut trades = Vec::new();
        let is_market = order.order_type == OrderType::Market;
        let mut empty_prices = Vec::new();
        let mut filled_resting: Vec<(u32, u64)> = Vec::new();
        let tick = self.tick;

        for (&price, level) in self.asks.levels.iter_mut() {
            if order.quantity <= f32::EPSILON {
                break;
            }
            if !is_market && price.into_inner() > order.price {
                break;
            }

            while order.quantity > f32::EPSILON && !level.orders.is_empty() {
                let resting = level.orders.front_mut().unwrap();
                let fill_qty = order.quantity.min(resting.quantity);

                trades.push(Trade {
                    buyer_id: order.agent_id,
                    seller_id: resting.agent_id,
                    price: price.into_inner(),
                    quantity: fill_qty,
                    tick,
                });

                order.quantity -= fill_qty;
                resting.quantity -= fill_qty;
                level.total_quantity -= fill_qty;

                if resting.quantity <= f32::EPSILON {
                    let done = level.orders.pop_front().unwrap();
                    filled_resting.push((done.agent_id, done.order_id));
                }
            }

            if level.orders.is_empty() {
                empty_prices.push(price);
            }
        }

        for price in empty_prices {
            self.asks.levels.remove(&price);
        }
        for (agent_id, order_id) in filled_resting {
            self.order_index.remove(&order_id);
            if let Some(orders) = self.agent_orders.get_mut(&agent_id) {
                orders.retain(|id| *id != order_id);
            }
        }

        trades
    }

    /// Match an incoming sell against resting bids (descending by price).
    fn match_sell_order(&mut self, order: &mut LobOrder) -> Vec<Trade> {
        let mut trades = Vec::new();
        let is_market = order.order_type == OrderType::Market;
        let mut empty_prices = Vec::new();
        let mut filled_resting: Vec<(u32, u64)> = Vec::new();
        let tick = self.tick;

        for (&price, level) in self.bids.levels.iter_mut().rev() {
            if order.quantity <= f32::EPSILON {
                break;
            }
            if !is_market && price.into_inner() < order.price {
                break;
            }

            while order.quantity > f32::EPSILON && !level.orders.is_empty() {
                let resting = level.orders.front_mut().unwrap();
                let fill_qty = order.quantity.min(resting.quantity);

                trades.push(Trade {
                    buyer_id: resting.agent_id,
                    seller_id: order.agent_id,
                    price: price.into_inner(),
                    quantity: fill_qty,
                    tick,
                });

                order.quantity -= fill_qty;
                resting.quantity -= fill_qty;
                level.total_quantity -= fill_qty;

                if resting.quantity <= f32::EPSILON {
                    let done = level.orders.pop_front().unwrap();
                    filled_resting.push((done.agent_id, done.order_id));
                }
            }

            if level.orders.is_empty() {
                empty_prices.push(price);
            }
        }

        for price in empty_prices {
            self.bids.levels.remove(&price);
        }
        for (agent_id, order_id) in filled_resting {
            self.order_index.remove(&order_id);
            if let Some(orders) = self.agent_orders.get_mut(&agent_id) {
                orders.retain(|id| *id != order_id);
            }
        }

        trades
    }

    /// Place a limit order on the book (no matching).
    fn rest_order(&mut self, order: LobOrder) {
        let price = OrderedFloat(order.price);
        let order_id = order.order_id;
        let agent_id = order.agent_id;
        let side = order.side;
        let qty = order.quantity;

        let book_side = match side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };

        let level = book_side
            .levels
            .entry(price)
            .or_insert_with(|| PriceLevel {
                orders: VecDeque::new(),
                total_quantity: 0.0,
            });
        level.total_quantity += qty;
        level.orders.push_back(order);

        self.order_index.insert(order_id, (side, price));
        self.agent_orders.entry(agent_id).or_default().push(order_id);
    }

    /// Full tick: cancel specified agents, then process market orders, then limit orders.
    pub fn process_tick(
        &mut self,
        cancel_agents: &[u32],
        market_orders: Vec<LobOrder>,
        limit_orders: Vec<LobOrder>,
        tick: u64,
    ) -> Vec<Trade> {
        self.tick = tick;
        let mut all_trades = Vec::new();

        for &agent_id in cancel_agents {
            self.cancel_agent(agent_id);
        }

        for order in market_orders {
            let trades = self.submit_order(order);
            all_trades.extend(trades);
        }

        for order in limit_orders {
            let trades = self.submit_order(order);
            all_trades.extend(trades);
        }

        if let Some(last) = all_trades.last() {
            self.last_price = last.price;
        }

        all_trades
    }

    /// Remove all orders placed before `min_tick`.
    pub fn expire_orders_before(&mut self, min_tick: u64) {
        Self::expire_side(
            &mut self.bids,
            min_tick,
            &mut self.agent_orders,
            &mut self.order_index,
        );
        Self::expire_side(
            &mut self.asks,
            min_tick,
            &mut self.agent_orders,
            &mut self.order_index,
        );
    }

    fn expire_side(
        book_side: &mut BookSide,
        min_tick: u64,
        agent_orders: &mut HashMap<u32, SmallVec<[u64; 4]>>,
        order_index: &mut HashMap<u64, (Side, OrderedFloat<f32>)>,
    ) {
        let mut empty_prices = Vec::new();
        for (&price, level) in book_side.levels.iter_mut() {
            level.orders.retain(|o| {
                if o.tick < min_tick {
                    order_index.remove(&o.order_id);
                    if let Some(orders) = agent_orders.get_mut(&o.agent_id) {
                        orders.retain(|id| *id != o.order_id);
                    }
                    false
                } else {
                    true
                }
            });
            level.total_quantity = level.orders.iter().map(|o| o.quantity).sum();
            if level.orders.is_empty() {
                empty_prices.push(price);
            }
        }
        for price in empty_prices {
            book_side.levels.remove(&price);
        }
    }

    /// Best bid/offer snapshot. Falls back to synthetic spread around last_price
    /// when either side is empty.
    pub fn bbo(&self) -> BBO {
        let half_spread = self.last_price * 0.001; // 10 bps fallback
        let (best_bid, best_bid_size) = self
            .bids
            .levels
            .iter()
            .next_back()
            .map(|(&p, l)| (p.into_inner(), l.total_quantity))
            .unwrap_or((self.last_price - half_spread, 0.0));
        let (best_ask, best_ask_size) = self
            .asks
            .levels
            .iter()
            .next()
            .map(|(&p, l)| (p.into_inner(), l.total_quantity))
            .unwrap_or((self.last_price + half_spread, 0.0));
        BBO {
            best_bid,
            best_bid_size,
            best_ask,
            best_ask_size,
            last_price: self.last_price,
            tick: self.tick,
            fair_value: self.last_price,
        }
    }

    pub fn spread(&self) -> f32 {
        let bbo = self.bbo();
        bbo.best_ask - bbo.best_bid
    }

    pub fn book_depth(&self) -> usize {
        self.bids.total_orders() + self.asks.total_orders()
    }

    pub fn bids_total_qty(&self) -> f32 {
        self.bids.total_quantity()
    }

    pub fn asks_total_qty(&self) -> f32 {
        self.asks.total_quantity()
    }

    /// Top N bid price levels: (price, total_quantity), best first.
    pub fn book_bids(&self, n: usize) -> Vec<(f32, f32)> {
        self.bids
            .levels
            .iter()
            .rev()
            .take(n)
            .map(|(&p, l)| (p.into_inner(), l.total_quantity))
            .collect()
    }

    /// Top N ask price levels: (price, total_quantity), best first.
    pub fn book_asks(&self, n: usize) -> Vec<(f32, f32)> {
        self.asks
            .levels
            .iter()
            .take(n)
            .map(|(&p, l)| (p.into_inner(), l.total_quantity))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limit_buy(agent_id: u32, price: f32, qty: f32, tick: u64) -> LobOrder {
        LobOrder {
            order_id: 0,
            agent_id,
            side: Side::Buy,
            price,
            quantity: qty,
            order_type: OrderType::Limit,
            tick,
        }
    }

    fn limit_sell(agent_id: u32, price: f32, qty: f32, tick: u64) -> LobOrder {
        LobOrder {
            order_id: 0,
            agent_id,
            side: Side::Sell,
            price,
            quantity: qty,
            order_type: OrderType::Limit,
            tick,
        }
    }

    fn market_buy(agent_id: u32, qty: f32, tick: u64) -> LobOrder {
        LobOrder {
            order_id: 0,
            agent_id,
            side: Side::Buy,
            price: f32::MAX,
            quantity: qty,
            order_type: OrderType::Market,
            tick,
        }
    }

    fn market_sell(agent_id: u32, qty: f32, tick: u64) -> LobOrder {
        LobOrder {
            order_id: 0,
            agent_id,
            side: Side::Sell,
            price: 0.0,
            quantity: qty,
            order_type: OrderType::Market,
            tick,
        }
    }

    #[test]
    fn test_basic_crossing() {
        let mut book = LimitOrderBook::new(100.0);
        // Post a sell that rests
        let trades1 = book.submit_order(limit_sell(1, 99.0, 3.0, 0));
        assert!(trades1.is_empty());

        // Aggressive buy crosses the resting sell
        let trades2 = book.submit_order(limit_buy(0, 101.0, 5.0, 0));
        assert_eq!(trades2.len(), 1);
        assert_eq!(trades2[0].buyer_id, 0);
        assert_eq!(trades2[0].seller_id, 1);
        assert_eq!(trades2[0].quantity, 3.0);
        // Trade at resting (ask) price, not midpoint
        assert!((trades2[0].price - 99.0).abs() < 0.01);

        // Remaining 2.0 rests on bids
        assert_eq!(book.book_depth(), 1);
        let bids = book.book_bids(10);
        assert_eq!(bids.len(), 1);
        assert!((bids[0].0 - 101.0).abs() < 0.01);
        assert!((bids[0].1 - 2.0).abs() < 0.01);
    }

    #[test]
    fn test_no_crossing() {
        let mut book = LimitOrderBook::new(100.0);
        let trades = book.process_tick(
            &[],
            vec![],
            vec![
                limit_buy(0, 99.0, 5.0, 0),
                limit_sell(1, 101.0, 3.0, 0),
            ],
            0,
        );
        assert!(trades.is_empty());
        assert_eq!(book.book_depth(), 2);
    }

    #[test]
    fn test_bbo_updates() {
        let mut book = LimitOrderBook::new(100.0);
        book.process_tick(
            &[],
            vec![],
            vec![
                limit_buy(0, 99.5, 10.0, 0),
                limit_sell(1, 100.5, 5.0, 0),
            ],
            0,
        );
        let bbo = book.bbo();
        assert!((bbo.best_bid - 99.5).abs() < 0.01);
        assert!((bbo.best_ask - 100.5).abs() < 0.01);
        assert!((bbo.best_bid_size - 10.0).abs() < 0.01);
        assert!((bbo.best_ask_size - 5.0).abs() < 0.01);
    }

    #[test]
    fn test_price_time_priority() {
        let mut book = LimitOrderBook::new(100.0);
        book.submit_order(limit_sell(1, 100.0, 2.0, 0));
        book.submit_order(limit_sell(2, 100.0, 3.0, 0));

        // Buy should match agent 1 first (earlier in queue)
        let trades = book.submit_order(limit_buy(0, 100.0, 4.0, 0));
        assert_eq!(trades.len(), 2);
        assert_eq!(trades[0].seller_id, 1);
        assert_eq!(trades[0].quantity, 2.0);
        assert_eq!(trades[1].seller_id, 2);
        assert_eq!(trades[1].quantity, 2.0);
    }

    #[test]
    fn test_market_order_walks_book() {
        let mut book = LimitOrderBook::new(100.0);
        book.submit_order(limit_sell(1, 99.0, 2.0, 0));
        book.submit_order(limit_sell(2, 100.0, 3.0, 0));

        let trades = book.submit_order(market_buy(0, 4.0, 0));
        assert_eq!(trades.len(), 2);
        assert!((trades[0].price - 99.0).abs() < 0.01);
        assert_eq!(trades[0].quantity, 2.0);
        assert!((trades[1].price - 100.0).abs() < 0.01);
        assert_eq!(trades[1].quantity, 2.0);

        // 1.0 remaining from ask at 100.0, unfilled market qty discarded
        assert_eq!(book.book_depth(), 1);
    }

    #[test]
    fn test_market_sell() {
        let mut book = LimitOrderBook::new(100.0);
        book.submit_order(limit_buy(1, 101.0, 2.0, 0));
        book.submit_order(limit_buy(2, 100.0, 3.0, 0));

        let trades = book.submit_order(market_sell(0, 4.0, 0));
        assert_eq!(trades.len(), 2);
        // Sells against highest bid first
        assert!((trades[0].price - 101.0).abs() < 0.01);
        assert_eq!(trades[0].quantity, 2.0);
        assert!((trades[1].price - 100.0).abs() < 0.01);
        assert_eq!(trades[1].quantity, 2.0);
    }

    #[test]
    fn test_cancel_agent() {
        let mut book = LimitOrderBook::new(100.0);
        book.submit_order(limit_buy(0, 99.0, 5.0, 0));
        book.submit_order(limit_sell(0, 101.0, 3.0, 0));
        book.submit_order(limit_buy(1, 98.0, 2.0, 0));

        assert_eq!(book.book_depth(), 3);
        book.cancel_agent(0);
        assert_eq!(book.book_depth(), 1);
    }

    #[test]
    fn test_expire_orders() {
        let mut book = LimitOrderBook::new(100.0);
        book.process_tick(
            &[],
            vec![],
            vec![
                limit_buy(0, 99.0, 5.0, 0),
                limit_sell(1, 101.0, 3.0, 0),
            ],
            0,
        );
        assert_eq!(book.book_depth(), 2);

        // Orders from tick 0 expire when min_tick = 1
        book.expire_orders_before(1);
        assert_eq!(book.book_depth(), 0);
    }

    #[test]
    fn test_expire_preserves_newer() {
        let mut book = LimitOrderBook::new(100.0);
        book.process_tick(
            &[],
            vec![],
            vec![limit_buy(0, 99.0, 5.0, 0)],
            0,
        );
        book.process_tick(
            &[],
            vec![],
            vec![limit_sell(1, 101.0, 3.0, 1)],
            1,
        );
        assert_eq!(book.book_depth(), 2);

        // Expire tick < 1: removes tick-0 order, keeps tick-1
        book.expire_orders_before(1);
        assert_eq!(book.book_depth(), 1);
        let asks = book.book_asks(10);
        assert_eq!(asks.len(), 1);
        assert!((asks[0].0 - 101.0).abs() < 0.01);
    }

    #[test]
    fn test_trade_at_resting_price() {
        let mut book = LimitOrderBook::new(100.0);
        book.submit_order(limit_sell(1, 99.0, 5.0, 0));

        // Aggressive buy at 105 trades at resting ask (99), not midpoint
        let trades = book.submit_order(limit_buy(0, 105.0, 3.0, 0));
        assert_eq!(trades.len(), 1);
        assert!((trades[0].price - 99.0).abs() < 0.01);
    }

    #[test]
    fn test_multi_level_matching() {
        let mut book = LimitOrderBook::new(100.0);
        book.submit_order(limit_sell(1, 99.0, 3.0, 0));
        book.submit_order(limit_sell(2, 100.0, 4.0, 0));
        book.submit_order(limit_sell(3, 101.0, 5.0, 0));

        // Buy at 100 crosses 99 and 100 but not 101
        let trades = book.submit_order(limit_buy(0, 100.0, 10.0, 0));
        assert_eq!(trades.len(), 2);
        assert!((trades[0].price - 99.0).abs() < 0.01);
        assert_eq!(trades[0].quantity, 3.0);
        assert!((trades[1].price - 100.0).abs() < 0.01);
        assert_eq!(trades[1].quantity, 4.0);

        // Remaining 3.0 rests on bids; ask at 101 still present
        assert_eq!(book.book_depth(), 2);
    }

    #[test]
    fn test_process_tick_cancel_then_quote() {
        let mut book = LimitOrderBook::new(100.0);
        // MM posts initial quotes
        book.process_tick(
            &[],
            vec![],
            vec![
                limit_buy(10, 99.0, 5.0, 0),
                limit_sell(10, 101.0, 5.0, 0),
            ],
            0,
        );
        assert_eq!(book.book_depth(), 2);

        // Next tick: cancel MM, post new quotes
        book.process_tick(
            &[10],
            vec![],
            vec![
                limit_buy(10, 99.5, 3.0, 1),
                limit_sell(10, 100.5, 3.0, 1),
            ],
            1,
        );
        assert_eq!(book.book_depth(), 2);
        let bbo = book.bbo();
        assert!((bbo.best_bid - 99.5).abs() < 0.01);
        assert!((bbo.best_ask - 100.5).abs() < 0.01);
    }
}
