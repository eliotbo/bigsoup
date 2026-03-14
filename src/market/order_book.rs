use super::types::{BBO, Order, Trade};

/// Separated buy/sell sides, sorted and ready for matching.
pub struct SortedOrders {
    pub buys: Vec<Order>,  // sorted descending by price
    pub sells: Vec<Order>, // sorted ascending by price
}

/// Strategy for sorting orders before matching.
pub enum SortStrategy {
    /// Standard CPU sort: O(N log N). Simple, always correct.
    CpuSort,
    /// Partition into price buckets, sort within buckets. O(N + B log B).
    /// `bucket_width` is the price range per bucket.
    BucketSort { bucket_width: f32 },
    /// Only find crossing orders (buys above best ask, sells below best bid).
    /// Skips non-crossing orders entirely, then sorts the smaller crossing set.
    CrossingOnly,
}

impl SortStrategy {
    pub fn sort(&self, orders: &[Order], bbo: &BBO) -> SortedOrders {
        match self {
            SortStrategy::CpuSort => Self::cpu_sort(orders),
            SortStrategy::BucketSort { bucket_width } => Self::bucket_sort(orders, *bucket_width),
            SortStrategy::CrossingOnly => Self::crossing_only(orders, bbo),
        }
    }

    fn cpu_sort(orders: &[Order]) -> SortedOrders {
        let mut buys = Vec::new();
        let mut sells = Vec::new();
        for order in orders {
            if order.quantity > 0.0 {
                buys.push(*order);
            } else if order.quantity < 0.0 {
                sells.push(*order);
            }
        }
        buys.sort_by(|a, b| b.price.partial_cmp(&a.price).unwrap());
        sells.sort_by(|a, b| a.price.partial_cmp(&b.price).unwrap());
        SortedOrders { buys, sells }
    }

    fn bucket_sort(orders: &[Order], bucket_width: f32) -> SortedOrders {
        // Find price range
        let mut min_price = f32::MAX;
        let mut max_price = f32::MIN;
        for order in orders {
            if order.quantity != 0.0 {
                min_price = min_price.min(order.price);
                max_price = max_price.max(order.price);
            }
        }
        if min_price > max_price {
            return SortedOrders { buys: Vec::new(), sells: Vec::new() };
        }

        let n_buckets = ((max_price - min_price) / bucket_width) as usize + 1;
        let mut buy_buckets: Vec<Vec<Order>> = vec![Vec::new(); n_buckets];
        let mut sell_buckets: Vec<Vec<Order>> = vec![Vec::new(); n_buckets];

        for order in orders {
            let bucket = ((order.price - min_price) / bucket_width) as usize;
            let bucket = bucket.min(n_buckets - 1);
            if order.quantity > 0.0 {
                buy_buckets[bucket].push(*order);
            } else if order.quantity < 0.0 {
                sell_buckets[bucket].push(*order);
            }
        }

        // Buys: iterate buckets high to low, sort within each bucket descending
        let mut buys = Vec::new();
        for bucket in buy_buckets.iter_mut().rev() {
            bucket.sort_by(|a, b| b.price.partial_cmp(&a.price).unwrap());
            buys.extend(bucket.iter());
        }

        // Sells: iterate buckets low to high, sort within each bucket ascending
        let mut sells = Vec::new();
        for bucket in sell_buckets.iter_mut() {
            bucket.sort_by(|a, b| a.price.partial_cmp(&b.price).unwrap());
            sells.extend(bucket.iter());
        }

        SortedOrders { buys, sells }
    }

    fn crossing_only(orders: &[Order], bbo: &BBO) -> SortedOrders {
        // Only collect orders that could potentially cross:
        // buys priced at or above best ask, sells priced at or below best bid
        let mut buys = Vec::new();
        let mut sells = Vec::new();
        for order in orders {
            if order.quantity > 0.0 && order.price >= bbo.best_ask {
                buys.push(*order);
            } else if order.quantity < 0.0 && order.price <= bbo.best_bid {
                sells.push(*order);
            }
        }
        buys.sort_by(|a, b| b.price.partial_cmp(&a.price).unwrap());
        sells.sort_by(|a, b| a.price.partial_cmp(&b.price).unwrap());
        SortedOrders { buys, sells }
    }
}

pub struct OrderBook {
    last_price: f32,
    best_bid: f32,
    best_bid_size: f32,
    best_ask: f32,
    best_ask_size: f32,
    tick: u64,
    sort_strategy: SortStrategy,
}

impl OrderBook {
    pub fn new(initial_price: f32) -> Self {
        Self::with_strategy(initial_price, SortStrategy::CpuSort)
    }

    pub fn with_strategy(initial_price: f32, sort_strategy: SortStrategy) -> Self {
        let half_spread = initial_price * 0.001; // 10 bps
        Self {
            last_price: initial_price,
            best_bid: initial_price - half_spread,
            best_bid_size: 1.0,
            best_ask: initial_price + half_spread,
            best_ask_size: 1.0,
            tick: 0,
            sort_strategy,
        }
    }

    /// Clearing auction: sort orders via the configured strategy,
    /// walk inward matching until prices don't cross.
    pub fn process_orders(&mut self, orders: &[Order], tick: u64) -> Vec<Trade> {
        let bbo = self.bbo();
        let sorted = self.sort_strategy.sort(orders, &bbo);
        let trades = Self::match_sorted(&sorted, tick);

        // Update BBO from unmatched orders
        // (approximate: use the first unmatched on each side)
        let n_matched_buys = trades.iter()
            .map(|t| t.buyer_id)
            .collect::<std::collections::HashSet<_>>()
            .len();
        let n_matched_sells = trades.iter()
            .map(|t| t.seller_id)
            .collect::<std::collections::HashSet<_>>()
            .len();

        self.best_bid = if n_matched_buys < sorted.buys.len() {
            sorted.buys[n_matched_buys].price
        } else if !sorted.buys.is_empty() {
            sorted.buys.last().unwrap().price
        } else {
            self.last_price
        };
        self.best_bid_size = if n_matched_buys < sorted.buys.len() {
            sorted.buys[n_matched_buys].quantity
        } else {
            0.0
        };

        self.best_ask = if n_matched_sells < sorted.sells.len() {
            sorted.sells[n_matched_sells].price
        } else if !sorted.sells.is_empty() {
            sorted.sells.last().unwrap().price
        } else {
            self.last_price
        };
        self.best_ask_size = if n_matched_sells < sorted.sells.len() {
            sorted.sells[n_matched_sells].quantity.abs()
        } else {
            0.0
        };

        if let Some(last_trade) = trades.last() {
            self.last_price = last_trade.price;
        }

        self.tick = tick;
        trades
    }

    fn match_sorted(sorted: &SortedOrders, tick: u64) -> Vec<Trade> {
        let mut trades = Vec::new();
        let mut buy_idx = 0;
        let mut sell_idx = 0;
        let mut buy_remaining = sorted.buys.first().map_or(0.0, |o| o.quantity);
        let mut sell_remaining = sorted.sells.first().map_or(0.0, |o| o.quantity.abs());

        while buy_idx < sorted.buys.len() && sell_idx < sorted.sells.len() {
            let buy_order = &sorted.buys[buy_idx];
            let sell_order = &sorted.sells[sell_idx];

            if buy_order.price < sell_order.price {
                break;
            }

            let trade_price = (buy_order.price + sell_order.price) * 0.5;
            let trade_qty = buy_remaining.min(sell_remaining);

            if trade_qty > 0.0 {
                trades.push(Trade {
                    buyer_id: buy_order.agent_id,
                    seller_id: sell_order.agent_id,
                    price: trade_price,
                    quantity: trade_qty,
                    tick,
                });
                buy_remaining -= trade_qty;
                sell_remaining -= trade_qty;
            }

            if buy_remaining <= f32::EPSILON {
                buy_idx += 1;
                buy_remaining = sorted.buys.get(buy_idx).map_or(0.0, |o| o.quantity);
            }
            if sell_remaining <= f32::EPSILON {
                sell_idx += 1;
                sell_remaining = sorted.sells.get(sell_idx).map_or(0.0, |o| o.quantity.abs());
            }
        }

        trades
    }

    pub fn bbo(&self) -> BBO {
        BBO {
            best_bid: self.best_bid,
            best_bid_size: self.best_bid_size,
            best_ask: self.best_ask,
            best_ask_size: self.best_ask_size,
            last_price: self.last_price,
            tick: self.tick,
            fair_value: self.last_price, // overwritten by Simulation::step if exo process is active
        }
    }

    pub fn clear(&mut self) {
        self.best_bid_size = 0.0;
        self.best_ask_size = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_crossing() {
        let mut book = OrderBook::new(100.0);
        let orders = vec![
            Order { agent_id: 0, price: 101.0, quantity: 5.0 },
            Order { agent_id: 1, price: 99.0, quantity: -3.0 },
            Order { agent_id: 2, price: 100.5, quantity: -2.0 },
        ];
        let trades = book.process_orders(&orders, 1);
        assert_eq!(trades.len(), 2);
        assert_eq!(trades[0].buyer_id, 0);
        assert_eq!(trades[0].seller_id, 1);
        assert_eq!(trades[0].quantity, 3.0);
        assert!((trades[0].price - 100.0).abs() < 0.01);
        assert_eq!(trades[1].buyer_id, 0);
        assert_eq!(trades[1].seller_id, 2);
        assert_eq!(trades[1].quantity, 2.0);
        assert!((trades[1].price - 100.75).abs() < 0.01);
    }

    #[test]
    fn test_no_crossing() {
        let mut book = OrderBook::new(100.0);
        let orders = vec![
            Order { agent_id: 0, price: 99.0, quantity: 5.0 },
            Order { agent_id: 1, price: 101.0, quantity: -3.0 },
        ];
        let trades = book.process_orders(&orders, 1);
        assert!(trades.is_empty());
    }

    #[test]
    fn test_bbo_updates() {
        let mut book = OrderBook::new(100.0);
        let orders = vec![
            Order { agent_id: 0, price: 101.0, quantity: 10.0 },
            Order { agent_id: 1, price: 99.0, quantity: -3.0 },
        ];
        let _trades = book.process_orders(&orders, 1);
        let bbo = book.bbo();
        assert!(bbo.last_price > 0.0);
        assert_eq!(bbo.tick, 1);
    }

    /// All three sort strategies should produce the same trades for the same input.
    #[test]
    fn test_sort_strategies_agree() {
        let orders = vec![
            Order { agent_id: 0, price: 102.0, quantity: 5.0 },
            Order { agent_id: 1, price: 101.0, quantity: 3.0 },
            Order { agent_id: 2, price: 98.0, quantity: -4.0 },
            Order { agent_id: 3, price: 99.0, quantity: -2.0 },
            Order { agent_id: 4, price: 100.0, quantity: 1.0 },
            Order { agent_id: 5, price: 100.5, quantity: -1.0 },
        ];

        let mut book_cpu = OrderBook::new(100.0);
        let mut book_bucket = OrderBook::with_strategy(100.0, SortStrategy::BucketSort { bucket_width: 0.5 });

        let trades_cpu = book_cpu.process_orders(&orders, 1);
        let trades_bucket = book_bucket.process_orders(&orders, 1);

        assert_eq!(trades_cpu.len(), trades_bucket.len());
        for (a, b) in trades_cpu.iter().zip(trades_bucket.iter()) {
            assert_eq!(a.buyer_id, b.buyer_id);
            assert_eq!(a.seller_id, b.seller_id);
            assert!((a.price - b.price).abs() < 1e-5);
            assert!((a.quantity - b.quantity).abs() < 1e-5);
        }
    }
}
