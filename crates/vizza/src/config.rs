use crate::{state::{LineOverlay, PositionOverlay, PriceLevelQuad}, zoom::LodLevel};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
}

impl Default for Theme {
    fn default() -> Self {
        Theme::Light
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ColorPalette {
    pub background: [f32; 3],
    pub viewport_bg: [f32; 3],
    pub grid_line: [f32; 4],
    pub candle_up_market: [f32; 3],
    pub candle_up_offhours: [f32; 3],
    pub candle_down_market: [f32; 3],
    pub candle_down_offhours: [f32; 3],
    pub wick: [f32; 3],
    pub volume: [f32; 3],
    pub text_primary: [u8; 4],
    pub text_secondary: [u8; 4],
}

impl ColorPalette {
    pub fn from_theme(theme: Theme) -> Self {
        match theme {
            Theme::Light => Self {
                background: [0.15, 0.15, 0.15],
                viewport_bg: [0.08, 0.08, 0.08],
                grid_line: [1.0, 1.0, 1.0, 0.15],
                candle_up_market: [0.2, 0.8, 0.3],
                candle_up_offhours: [0.4, 0.6, 0.45],
                candle_down_market: [0.9, 0.2, 0.2],
                candle_down_offhours: [0.7, 0.4, 0.4],
                wick: [0.75, 0.75, 0.75],
                volume: [0.0, 0.4, 1.0],
                text_primary: [220, 220, 220, 255],
                text_secondary: [160, 160, 160, 180],
            },
            Theme::Dark => Self {
                background: [0.08, 0.08, 0.08],
                viewport_bg: [0.05, 0.05, 0.05],
                grid_line: [1.0, 1.0, 1.0, 0.12],
                candle_up_market: [0.2, 0.9, 0.4],
                candle_up_offhours: [0.05, 0.3, 0.1],
                candle_down_market: [1.0, 0.25, 0.25],
                candle_down_offhours: [0.35, 0.05, 0.05],
                wick: [0.75, 0.75, 0.75],
                volume: [0.2, 0.5, 1.0],
                text_primary: [220, 220, 220, 255],
                text_secondary: [160, 160, 160, 180],
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub default_lod_level: LodLevel,
    pub data_paths: Vec<String>,
    pub view_settings: ViewSettings,
    pub window_width: u32,
    pub window_height: u32,
    pub grid_rows: usize,
    pub grid_cols: usize,
    pub allow_missing_history: bool,
    pub position_overlays: Vec<Vec<PositionOverlay>>,
    pub price_level_quads: Vec<Vec<PriceLevelQuad>>,
    pub line_overlays: Vec<Vec<LineOverlay>>,
    pub tickers: Vec<Option<String>>,
    pub titles: Vec<Option<String>>,
    pub bar_width_px: u16,
    /// Initial viewport left edge timestamp (Unix seconds).
    /// If None, defaults to showing most recent data.
    pub initial_left_ts: Option<i64>,
    /// Per-viewport initial left edge timestamps (Unix seconds).
    /// One entry per viewport in row-major order. None means use default (most recent data).
    pub initial_left_times: Vec<Option<i64>>,
    pub theme: Theme,
}

#[derive(Debug, Clone)]
pub struct ViewSettings {
    pub auto_y_scale: bool,
    pub show_volume_bars: bool,
}

impl Default for ViewSettings {
    fn default() -> Self {
        Self {
            auto_y_scale: true,
            show_volume_bars: true,
        }
    }
}

const BASE_PATH: &str = "/media/data10t/databento/";
const STOCK_PATH: &str = "tiny_test/FAST/consolidated/";

impl Default for Config {
    fn default() -> Self {
        // Build full paths: BASE_PATH + STOCK_PATH + filename
        let base_path = BASE_PATH;
        let stock_path = STOCK_PATH;

        Self {
            default_lod_level: LodLevel::D1,
            // Using FAST data with splits.json for automatic split adjustment
            data_paths: vec![
                format!(
                    "{}{}2025-01-15_to_2025-10-03-ohlcv-1m.nohlcv",
                    base_path, stock_path
                ),
                format!(
                    "{}{}2025-01-15_to_2025-10-03-ohlcv-1m.nohlcv",
                    base_path, stock_path
                ),
                format!(
                    "{}{}2025-01-15_to_2025-10-03-ohlcv-1m.nohlcv",
                    base_path, stock_path
                ),
                format!(
                    "{}{}2025-01-15_to_2025-10-03-ohlcv-1m.nohlcv",
                    base_path, stock_path
                ),
            ],
            view_settings: ViewSettings::default(),
            window_width: 1200,
            window_height: 1200,
            grid_rows: 2,
            grid_cols: 2,
            allow_missing_history: false,
            position_overlays: Vec::new(),
            price_level_quads: Vec::new(),
            line_overlays: Vec::new(),
            tickers: Vec::new(),
            titles: Vec::new(),
            bar_width_px: 3,
            initial_left_ts: None,
            initial_left_times: Vec::new(),
            theme: Theme::default(),
        }
    }
}
