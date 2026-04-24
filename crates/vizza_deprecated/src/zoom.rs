use crate::config::Config;

// Constant for gap between bars
const MIN_GAP: f64 = 1.0; // Minimum gap between bars

/// Represents a single Level of Detail configuration
/// Each level defines a time granularity and its display label
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LodLevel {
    S1,     // 1 second
    S5,     // 5 seconds
    S15,    // 15 seconds
    S30,    // 30 seconds
    M1,     // 1 minute
    M5,     // 5 minutes
    M15,    // 15 minutes
    M30,    // 30 minutes
    H1,     // 1 hour
    H4,     // 4 hours
    D1,     // 1 day
    W1,     // 1 week
    Month1, // 1 month
}

impl LodLevel {
    /// Returns the duration in seconds for this LOD level
    pub fn seconds(&self) -> f64 {
        match self {
            LodLevel::S1 => 1.0,
            LodLevel::S5 => 5.0,
            LodLevel::S15 => 15.0,
            LodLevel::S30 => 30.0,
            LodLevel::M1 => 60.0,
            LodLevel::M5 => 300.0,
            LodLevel::M15 => 900.0,
            LodLevel::M30 => 1800.0,
            LodLevel::H1 => 3600.0,
            LodLevel::H4 => 14400.0,
            LodLevel::D1 => 86400.0,
            LodLevel::W1 => 604800.0,        // 7 days
            LodLevel::Month1 => 2_629_746.0, // Approx 1 month (30.44 days)
        }
    }

    /// Returns the human-readable label for this LOD level
    pub fn label(&self) -> &'static str {
        match self {
            LodLevel::S1 => "1s",
            LodLevel::S5 => "5s",
            LodLevel::S15 => "15s",
            LodLevel::S30 => "30s",
            LodLevel::M1 => "1m",
            LodLevel::M5 => "5m",
            LodLevel::M15 => "15m",
            LodLevel::M30 => "30m",
            LodLevel::H1 => "1h",
            LodLevel::H4 => "4h",
            LodLevel::D1 => "1d",
            LodLevel::W1 => "1w",
            LodLevel::Month1 => "1month",
        }
    }

    /// Returns all available LOD levels in order from finest to coarsest
    pub fn all_levels() -> &'static [LodLevel] {
        &[
            LodLevel::S1,
            LodLevel::S5,
            LodLevel::S15,
            LodLevel::S30,
            LodLevel::M1,
            LodLevel::M5,
            LodLevel::M15,
            LodLevel::M30,
            LodLevel::H1,
            LodLevel::H4,
            LodLevel::D1,
            LodLevel::W1,
            LodLevel::Month1,
        ]
    }

    /// Find the closest LOD level for a given interval in seconds
    pub fn from_seconds(secs: f64) -> LodLevel {
        let all = Self::all_levels();
        all.iter()
            .min_by_key(|level| ((level.seconds() - secs).abs() * 1000.0) as i64)
            .copied()
            .unwrap_or(LodLevel::M1)
    }
}

/// Zoom state for a viewport
#[derive(Debug, Clone, Copy)]
pub struct ZoomX {
    pub bar_width_px: u16, // Default bar width in pixels (adjustable)
    pub current_lod_level: LodLevel,
    last_lod_level: LodLevel,
    min_lod_level: LodLevel, // Minimum (finest) LOD level allowed based on data
}

impl Default for ZoomX {
    fn default() -> Self {
        let config = Config::default();
        Self {
            bar_width_px: 3,
            current_lod_level: config.default_lod_level,
            last_lod_level: LodLevel::W1,
            min_lod_level: LodLevel::M1, // Default to 1 minute
        }
    }
}

impl ZoomX {
    /// Set the minimum (finest) LOD level based on data granularity
    pub fn set_min_lod_from_interval(&mut self, interval_secs: u64) {
        self.min_lod_level = LodLevel::from_seconds(interval_secs as f64);

        // Ensure current LOD is not finer than min LOD
        let all_levels = LodLevel::all_levels();
        let min_idx = all_levels
            .iter()
            .position(|&l| l == self.min_lod_level)
            .unwrap_or(0);
        let current_idx = all_levels
            .iter()
            .position(|&l| l == self.current_lod_level)
            .unwrap_or(0);

        if current_idx < min_idx {
            self.current_lod_level = self.min_lod_level;
        }
    }

    /// Handle LOD change from Ctrl+scroll while respecting which levels have data.
    /// scroll_delta > 0 means finer detail, < 0 means coarser detail.
    /// If no data exists in the desired direction, the LOD remains unchanged.
    pub fn handle_lod_change(&mut self, scroll_delta: f64, available_lods: &[LodLevel]) {
        if scroll_delta == 0.0 || available_lods.is_empty() {
            return;
        }

        let all_levels = LodLevel::all_levels();
        let current_idx = all_levels
            .iter()
            .position(|&l| l == self.current_lod_level)
            .unwrap_or(0);

        let direction: isize = if scroll_delta > 0.0 { -1 } else { 1 };
        let mut idx = current_idx as isize + direction;

        while idx >= 0 && (idx as usize) < all_levels.len() {
            let candidate = all_levels[idx as usize];
            if available_lods.contains(&candidate) {
                self.last_lod_level = self.current_lod_level;
                self.current_lod_level = candidate;
                return;
            }
            idx += direction;
        }

        // No available LOD in the requested direction; keep the current level.
    }

    /// Decrease bar width by 2 pixels (down to a minimum of 1).
    /// Returns true if the width changed.
    pub fn decrease_bar_width(&mut self) -> bool {
        let current = self.bar_width_px as i16;
        let desired = current - 2;
        let mut clamped = desired.clamp(1, 7);
        // Ensure width stays odd to keep gaps centered
        if clamped % 2 == 0 {
            clamped = (clamped - 1).max(1);
        }

        let new_width = clamped as u16;
        if new_width != self.bar_width_px {
            self.bar_width_px = new_width;
            true
        } else {
            false
        }
    }

    /// Increase bar width by 2 pixels (up to a maximum of 7).
    /// Returns true if the width changed.
    pub fn increase_bar_width(&mut self) -> bool {
        let current = self.bar_width_px as i16;
        let desired = current + 2;
        let mut clamped = desired.clamp(1, 7);
        if clamped % 2 == 0 {
            clamped = (clamped - 1).max(1);
        }

        let new_width = clamped as u16;
        if new_width != self.bar_width_px {
            self.bar_width_px = new_width;
            true
        } else {
            false
        }
    }

    /// Get the minimum LOD level supported by historical data
    pub fn min_lod_level(&self) -> LodLevel {
        self.min_lod_level
    }

    pub fn get_num_bars_in_viewport(&self, viewport_width_in_pixels: f64) -> u32 {
        // Each bar is 3 pixels wide with 1 pixel gap
        let bar_width = self.bar_width_px as f64;
        let total_bar_space = bar_width + MIN_GAP;
        (viewport_width_in_pixels / total_bar_space).floor() as u32
    }
}
