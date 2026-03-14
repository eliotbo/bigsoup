use std::sync::{Arc, Mutex};
use std::time::Duration;

use lod::aggregator::StreamingAggregator;
use lod::traits::QuoteLike;
use lod::{LevelStore, LiveEngine, LiveSnapshot, PlotCandle, PlotTrade};

use crate::live::{LiveDataSource, SyntheticDataSource};

/// Marker used to detect when live snapshot data has changed meaningfully.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SnapshotMarker {
    completed_len: usize,
    completed_last_ts: i64,
    last_trade_ns: i64,
    open_signature: Option<(f32, f32, f32, f32, f32)>,
}

impl SnapshotMarker {
    fn from_snapshot(snapshot: &LiveSnapshot) -> Self {
        let completed_len = snapshot.completed.len();
        let completed_last_ts = snapshot
            .completed
            .last()
            .map(|candle| candle.ts)
            .unwrap_or(0);
        let open_signature = snapshot.open_bar.map(|candle| {
            (
                candle.open,
                candle.high,
                candle.low,
                candle.close,
                candle.volume,
            )
        });

        SnapshotMarker {
            completed_len,
            completed_last_ts,
            last_trade_ns: snapshot.last_trade_ns,
            open_signature,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum PriceTrend {
    Up,
    Down,
    Flat,
}

/// VWAP marker data for rendering overlays.
#[derive(Clone, Copy, Debug)]
pub struct LiveVwapMarker {
    pub price: f32,
    pub volume: f32,
    pub trend: PriceTrend,
}

/// Data prepared for rendering in the live overlay path.
#[derive(Clone)]
pub struct LiveRenderData {
    pub completed: Arc<[PlotCandle]>,
    pub open_bar: Option<PlotCandle>,
    pub last_trade_ns: i64,
    pub vwap_marker: Option<LiveVwapMarker>,
}

#[derive(Clone)]
pub enum LiveSnapshotState {
    Unavailable,
    Unchanged,
    Updated(LiveRenderData),
}

pub struct LiveDataManager {
    engine: Arc<Mutex<LiveEngine>>,
    data_source: Box<dyn LiveDataSource>,
    level_store: Arc<Mutex<LevelStore>>,
    current_time_ns: i64,
    intervals_secs: Vec<u64>,
    primary_interval_secs: u64,
    consolidation_timer: Duration,
    consolidation_interval: Duration,
    keep_last_completed: usize,
    last_marker: Option<SnapshotMarker>,
    drop_logged: bool,
    snapshot_interval: Duration,
    time_since_snapshot: Duration,
    recent_trades: Vec<PlotTrade>,
    last_vwap: Option<f64>,
    last_vwap_volume: f64,
    last_trend: PriceTrend,

    // Today-so-far backfill support
    today_so_far_enabled: bool,
    backfill_completed: bool,
    backfill_end_ns: Option<i64>,
}

impl LiveDataManager {
    pub fn new(level_store: Arc<Mutex<LevelStore>>) -> Self {
        Self::with_data_source(level_store, Box::new(SyntheticDataSource::new()), "UNKNOWN")
    }

    /// Create a new LiveDataManager with a custom data source.
    pub fn with_data_source(
        level_store: Arc<Mutex<LevelStore>>,
        mut data_source: Box<dyn LiveDataSource>,
        ticker: &str,
    ) -> Self {
        Self::with_data_source_and_options(level_store, data_source, ticker, false)
    }

    /// Create a new LiveDataManager with a custom data source and options.
    pub fn with_data_source_and_options(
        level_store: Arc<Mutex<LevelStore>>,
        mut data_source: Box<dyn LiveDataSource>,
        ticker: &str,
        today_so_far_enabled: bool,
    ) -> Self {
        // Use 5 seconds as the finest granularity for live data
        // All coarser intervals (15s, 30s, 1m, etc.) are built from 5s bars
        let primary_interval_secs = 5;
        let intervals_secs = vec![5, 15, 30, 60, 300, 900, 1800, 3600];

        // Initialize the data source with historical context
        {
            let store = level_store.lock().expect("level_store mutex poisoned");
            if let Err(e) = data_source.initialize(&store, ticker) {
                eprintln!("Failed to initialize data source: {}", e);
            }
        }

        let base_epoch_ns = {
            let store = level_store.lock().expect("level_store mutex poisoned");

            // Try to find the last timestamp from any available interval
            // Try primary first, then fall back to coarser intervals
            let mut last_ts = None;
            for &interval in &[primary_interval_secs, 60, 300, 900] {
                if let Some(candles) = store.get(interval) {
                    if let Some(last) = candles.last() {
                        let interval_ns = (interval as i64) * 1_000_000_000;
                        last_ts = Some(last.ts.saturating_add(interval_ns));
                        break;
                    }
                }
            }

            last_ts.unwrap_or_else(|| {
                // No historical data - start from current time
                chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
            })
        };

        let engine = LiveEngine::new(
            intervals_secs.clone(),
            base_epoch_ns,
            100_000_000, // 100ms out-of-order window
            14,          // 16k capacity ring buffer
            256,
        );

        LiveDataManager {
            engine: Arc::new(Mutex::new(engine)),
            data_source,
            level_store,
            current_time_ns: base_epoch_ns,
            intervals_secs,
            primary_interval_secs,
            consolidation_timer: Duration::ZERO,
            consolidation_interval: Duration::from_secs(150),
            keep_last_completed: 200,
            last_marker: None,
            drop_logged: false,
            snapshot_interval: Duration::from_millis(150),
            time_since_snapshot: Duration::ZERO,
            recent_trades: Vec::with_capacity(1024),
            last_vwap: None,
            last_vwap_volume: 0.0,
            last_trend: PriceTrend::Flat,
            today_so_far_enabled,
            backfill_completed: false,
            backfill_end_ns: None,
        }
    }

    /// Advance the live engine by `dt`, ingesting trades and draining hot buffers.
    pub fn update(&mut self, dt: Duration) -> bool {
        let mut data_changed = false;
        let delta_ns_u128 = dt.as_nanos();
        let delta_ns = if delta_ns_u128 > i64::MAX as u128 {
            i64::MAX
        } else {
            delta_ns_u128 as i64
        };
        self.current_time_ns = self.current_time_ns.saturating_add(delta_ns);

        let trades = self.data_source.get_trades(self.current_time_ns);
        if !trades.is_empty() {
            data_changed = true;
        }

        if let Ok(mut engine) = self.engine.lock() {
            if !trades.is_empty() {
                self.recent_trades.extend(trades.iter().copied());

                for trade in trades {
                    if !engine.ingest_trade(trade) && !self.drop_logged {
                        eprintln!("live buffer overflow: dropping trades");
                        self.drop_logged = true;
                    }
                }
            }

            engine.process();
        }

        self.consolidation_timer += dt;
        if self.consolidation_timer >= self.consolidation_interval {
            self.consolidation_timer = Duration::ZERO;
            self.consolidate_to_cold();
        }

        self.time_since_snapshot = self.time_since_snapshot.saturating_add(dt);
        if self.time_since_snapshot >= self.snapshot_interval {
            data_changed = true;
        }

        data_changed
    }

    fn consolidate_to_cold(&mut self) {
        let mut drained_sets: Vec<(u64, Vec<PlotCandle>)> = Vec::new();

        if let Ok(mut engine) = self.engine.lock() {
            for &interval in &self.intervals_secs {
                let drained = engine.drain_old(interval, self.keep_last_completed);
                if !drained.is_empty() {
                    drained_sets.push((interval, drained));
                }
            }
        }

        if drained_sets.is_empty() {
            return;
        }

        if let Ok(mut store) = self.level_store.lock() {
            for (interval, drained) in drained_sets {
                store.append(interval, &drained, true);
            }
        }
    }

    /// Prepare data for live rendering for the requested interval.
    pub fn prepare_render(&mut self, interval_secs: u64) -> LiveSnapshotState {
        let snapshot_opt = {
            let engine = self.engine.lock().expect("live engine mutex poisoned");
            engine.snapshot(interval_secs)
        };

        let Some(snapshot) = snapshot_opt else {
            self.last_marker = None;
            self.recent_trades.clear();
            return LiveSnapshotState::Unavailable;
        };

        if snapshot.completed.is_empty() && snapshot.open_bar.is_none() {
            self.last_marker = None;
            self.recent_trades.clear();
            return LiveSnapshotState::Unavailable;
        }

        let marker = SnapshotMarker::from_snapshot(&snapshot);
        let force_refresh = self.time_since_snapshot >= self.snapshot_interval;

        if self.last_marker.as_ref() == Some(&marker) && !force_refresh {
            return LiveSnapshotState::Unchanged;
        }

        self.last_marker = Some(marker);

        let mut vwap_marker = None;

        if force_refresh && !self.recent_trades.is_empty() {
            let mut total_volume = 0.0f64;
            let mut weighted_price = 0.0f64;

            for trade in &self.recent_trades {
                let size = trade.size as f64;
                total_volume += size;
                weighted_price += trade.price as f64 * size;
            }

            if total_volume > 0.0 {
                let vwap = weighted_price / total_volume;

                let trend = match self.last_vwap {
                    Some(prev) if vwap > prev => PriceTrend::Up,
                    Some(prev) if vwap < prev => PriceTrend::Down,
                    Some(_) => PriceTrend::Flat,
                    None => PriceTrend::Flat,
                };

                self.last_vwap = Some(vwap);
                self.last_vwap_volume = total_volume;
                self.last_trend = trend;

                vwap_marker = Some(LiveVwapMarker {
                    price: vwap as f32,
                    volume: total_volume as f32,
                    trend,
                });
            }

            self.recent_trades.clear();
            self.time_since_snapshot = Duration::ZERO;
        } else if let Some(vwap) = self.last_vwap {
            vwap_marker = Some(LiveVwapMarker {
                price: vwap as f32,
                volume: self.last_vwap_volume as f32,
                trend: self.last_trend,
            });
        }

        LiveSnapshotState::Updated(LiveRenderData {
            completed: snapshot.completed,
            open_bar: snapshot.open_bar,
            last_trade_ns: snapshot.last_trade_ns,
            vwap_marker,
        })
    }

    /// Convenience accessor for the primary interval (used for consolidation into the cold store).
    pub fn primary_interval_secs(&self) -> u64 {
        self.primary_interval_secs
    }

    /// Initialize with today-so-far backfill if enabled and supported.
    ///
    /// This should be called after construction but before starting the update loop.
    /// It will attempt to fetch and inject historical intraday bars from market open
    /// to the current time.
    pub fn initialize_with_backfill(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        use crate::live::backfill::BackfillRequest;
        use chrono::{Datelike, TimeZone, Utc};

        if !self.today_so_far_enabled {
            return Ok(());
        }

        if self.backfill_completed {
            return Ok(()); // Already done
        }

        // Try to get the backfill source
        let backfill_source = self.data_source.as_backfill_source();

        if backfill_source.is_none() {
            eprintln!("⚠ Today-so-far requested but data source doesn't support backfill");
            self.backfill_completed = true; // Mark as completed to avoid repeated attempts
            return Ok(());
        }

        let backfill_source = backfill_source.unwrap();

        // Calculate market open time (9:30 AM ET)
        // Use New_York timezone to properly handle EDT/EST transitions
        use chrono_tz::America::New_York;
        let now = Utc::now();
        let now_et = now.with_timezone(&New_York);

        // Create 9:30 AM ET for today
        let market_open_et = New_York
            .with_ymd_and_hms(now_et.year(), now_et.month(), now_et.day(), 9, 30, 0)
            .single()
            .unwrap_or(now_et);

        let market_open = market_open_et.with_timezone(&Utc);

        let market_open_ns = market_open.timestamp_nanos_opt().unwrap_or(0);
        let now_ns = now.timestamp_nanos_opt().unwrap_or(0);

        eprintln!("✓ Requesting today-so-far backfill from market open to now");
        eprintln!(
            "  Market open: {} ({} UTC)",
            market_open_et.format("%Y-%m-%d %H:%M:%S ET"),
            market_open.format("%Y-%m-%d %H:%M:%S")
        );
        eprintln!(
            "  Current time: {} ({} UTC)",
            now_et.format("%Y-%m-%d %H:%M:%S ET"),
            now.format("%Y-%m-%d %H:%M:%S")
        );

        // Request historical bars
        let request = BackfillRequest::new(
            market_open_ns,
            now_ns,
            backfill_source.recommended_backfill_bar_size(),
            "UNKNOWN".to_string(), // TODO: Pass ticker through
        );

        match backfill_source.request_historical_bars(request) {
            Ok(response) => {
                eprintln!("✓ Received {} bars from backfill", response.bar_count());

                if !response.is_empty() {
                    // Inject the intraday_replay bars (bypasses LiveEngine ring buffer)
                    self.inject_intraday_replay_bars(response.bars)?;
                    self.backfill_end_ns = Some(response.actual_end_ns);
                    eprintln!("✓ Backfill injection completed successfully");
                } else {
                    eprintln!("⚠ Backfill returned no bars (market may be closed)");
                }

                self.backfill_completed = true;
                Ok(())
            }
            Err(e) => {
                eprintln!("⚠ Backfill request failed: {}", e);
                self.backfill_completed = true; // Mark as completed to avoid retry loops
                Err(e.into())
            }
        }
    }

    /// Inject intraday replay bars (e.g., today-so-far) directly into level_store.
    ///
    /// This bypasses the LiveEngine ring buffer and goes directly to level_store,
    /// preserving all historical bars for panning. This is the correct pipeline for
    /// pre-aggregated bars from IBKR or similar sources.
    ///
    /// Pipeline: IBKR 5s bars → level_store → coarser intervals → level_store
    fn inject_intraday_replay_bars(
        &mut self,
        bars: Vec<PlotCandle>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if bars.is_empty() {
            return Ok(());
        }

        eprintln!(
            "Injecting {} intraday_replay bars directly to level_store (bypassing LiveEngine ring buffer)",
            bars.len()
        );

        // Track the end time of the backfill for time synchronization
        let backfill_end_ns = bars.last().map(|b| b.ts);

        // Step 1: Inject primary interval bars (5s) directly into level_store
        if let Ok(mut store) = self.level_store.lock() {
            store.append(self.primary_interval_secs, &bars, false);
            eprintln!(
                "  → Appended {} bars to level_store for {}s interval",
                bars.len(),
                self.primary_interval_secs
            );
        }

        // Step 2: Build coarser intervals from the 5s bars using aggregator
        // Filter out primary interval to avoid re-aggregating it
        let coarser_intervals: Vec<u64> = self
            .intervals_secs
            .iter()
            .copied()
            .filter(|&i| i > self.primary_interval_secs)
            .collect();

        if !coarser_intervals.is_empty() {
            eprintln!("  → Building coarser intervals: {:?}", coarser_intervals);

            let mut aggregator =
                StreamingAggregator::new(self.primary_interval_secs, coarser_intervals.clone());

            // Feed all 5s bars into the aggregator
            for bar in &bars {
                aggregator.push(bar as &dyn QuoteLike);
            }

            // Finalize and get the aggregated levels
            let batch = aggregator.seal();

            // Step 3: Append coarser interval bars to level_store
            if let Ok(mut store) = self.level_store.lock() {
                for (interval, coarse_bars) in batch.levels {
                    if !coarse_bars.is_empty() {
                        store.append(interval, &coarse_bars, false);
                        eprintln!(
                            "  → Appended {} bars to level_store for {}s interval",
                            coarse_bars.len(),
                            interval
                        );
                    }
                }
            }
        }

        // Update current_time_ns to the end of backfill so live trades start from the correct time
        if let Some(end_ns) = backfill_end_ns {
            let bar_size_ns = (self.primary_interval_secs as i64) * 1_000_000_000;
            self.current_time_ns = end_ns + bar_size_ns;
            eprintln!(
                "  → Updated current_time_ns to {} (backfill end + {}s)",
                self.current_time_ns, self.primary_interval_secs
            );
        }

        eprintln!(
            "✓ Intraday replay injection completed - all {} bars preserved for panning",
            bars.len()
        );

        Ok(())
    }

    /// Check if today-so-far backfill is enabled.
    pub fn is_today_so_far_enabled(&self) -> bool {
        self.today_so_far_enabled
    }

    /// Check if backfill has been completed.
    pub fn is_backfill_completed(&self) -> bool {
        self.backfill_completed
    }
}

impl Default for LiveDataManager {
    fn default() -> Self {
        Self::new(Arc::new(Mutex::new(LevelStore::new())))
    }
}
