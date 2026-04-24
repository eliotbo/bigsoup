// Comprehensive test suite to ensure refactor safety
// These tests validate critical functionality before and after refactoring

use vizza::time_spacing::{
    TimeTickSpacing, TimeUnit, candidate_steps, choose_time_unit, select_time_spacing,
};
use vizza::zoom::{LodLevel, ZoomX};

// ============================================================================
// ZOOM TESTS
// ============================================================================

#[test]
fn test_lod_level_seconds() {
    assert_eq!(LodLevel::S1.seconds(), 1.0);
    assert_eq!(LodLevel::M1.seconds(), 60.0);
    assert_eq!(LodLevel::M5.seconds(), 300.0);
    assert_eq!(LodLevel::H1.seconds(), 3600.0);
    assert_eq!(LodLevel::D1.seconds(), 86400.0);
    assert_eq!(LodLevel::W1.seconds(), 604800.0);
}

#[test]
fn test_lod_level_labels() {
    assert_eq!(LodLevel::S1.label(), "1s");
    assert_eq!(LodLevel::M1.label(), "1m");
    assert_eq!(LodLevel::M5.label(), "5m");
    assert_eq!(LodLevel::H1.label(), "1h");
    assert_eq!(LodLevel::D1.label(), "1d");
    assert_eq!(LodLevel::W1.label(), "1w");
    assert_eq!(LodLevel::Month1.label(), "1month");
}

#[test]
fn test_lod_from_seconds_exact_match() {
    assert_eq!(LodLevel::from_seconds(1.0), LodLevel::S1);
    assert_eq!(LodLevel::from_seconds(60.0), LodLevel::M1);
    assert_eq!(LodLevel::from_seconds(300.0), LodLevel::M5);
    assert_eq!(LodLevel::from_seconds(3600.0), LodLevel::H1);
}

#[test]
fn test_lod_from_seconds_closest_match() {
    // Should pick closest LOD level
    assert_eq!(LodLevel::from_seconds(70.0), LodLevel::M1); // Closer to 60 than 300
    assert_eq!(LodLevel::from_seconds(400.0), LodLevel::M5); // Closer to 300 than 900
}

#[test]
fn test_zoom_default_state() {
    let zoom = ZoomX::default();
    assert_eq!(zoom.bar_width_px, 3);
    assert_eq!(zoom.current_lod_level, LodLevel::M1);
}

#[test]
fn test_zoom_set_min_lod_from_interval() {
    let mut zoom = ZoomX::default();

    // Set min LOD to 5 minutes
    zoom.set_min_lod_from_interval(300);
    // Current LOD should update if it was finer
    // Default is M1 (60s), which is finer than M5 (300s)
    assert_eq!(zoom.current_lod_level, LodLevel::M5);
}

#[test]
fn test_zoom_lod_change_finer() {
    let mut zoom = ZoomX::default();
    zoom.current_lod_level = LodLevel::M5;
    zoom.set_min_lod_from_interval(60);

    // Scroll positive (finer detail)
    zoom.handle_lod_change(1.0, LodLevel::all_levels());
    assert_eq!(zoom.current_lod_level, LodLevel::M1);
}

#[test]
fn test_zoom_lod_change_coarser() {
    let mut zoom = ZoomX::default();
    zoom.current_lod_level = LodLevel::M1;

    // Scroll negative (coarser detail)
    zoom.handle_lod_change(-1.0, LodLevel::all_levels());
    assert_eq!(zoom.current_lod_level, LodLevel::M5);
}

#[test]
fn test_zoom_lod_change_skips_missing_coarser_levels() {
    let mut zoom = ZoomX::default();
    zoom.current_lod_level = LodLevel::M1;

    let available = [LodLevel::M1, LodLevel::H1];
    zoom.handle_lod_change(-1.0, &available);

    assert_eq!(zoom.current_lod_level, LodLevel::H1);
}

#[test]
fn test_zoom_lod_change_skips_missing_finer_levels() {
    let mut zoom = ZoomX::default();
    zoom.current_lod_level = LodLevel::M1;

    let available = [LodLevel::S1, LodLevel::M1];
    zoom.handle_lod_change(1.0, &available);

    assert_eq!(zoom.current_lod_level, LodLevel::S1);
}

#[test]
fn test_zoom_lod_change_no_available_levels_in_direction() {
    let mut zoom = ZoomX::default();
    zoom.current_lod_level = LodLevel::M1;

    let available = [LodLevel::M1];
    zoom.handle_lod_change(-1.0, &available);

    assert_eq!(zoom.current_lod_level, LodLevel::M1);
}

#[test]
fn test_zoom_lod_change_respects_min_level() {
    let mut zoom = ZoomX::default();
    zoom.current_lod_level = LodLevel::M1;
    zoom.set_min_lod_from_interval(60); // Min is M1

    // Try to go finer - should stay at M1
    zoom.handle_lod_change(1.0, LodLevel::all_levels());
    assert_eq!(zoom.current_lod_level, LodLevel::M1);
}

#[test]
fn test_zoom_num_bars_in_viewport() {
    let zoom = ZoomX::default();
    // With 3px bars + 1px gap = 4px per bar
    let viewport_width = 800.0;
    let num_bars = zoom.get_num_bars_in_viewport(viewport_width);
    assert_eq!(num_bars, 200); // 800 / 4 = 200
}

#[test]
fn test_zoom_num_bars_different_widths() {
    let zoom = ZoomX::default();

    // Test various viewport widths
    assert_eq!(zoom.get_num_bars_in_viewport(400.0), 100);
    assert_eq!(zoom.get_num_bars_in_viewport(1200.0), 300);
    assert_eq!(zoom.get_num_bars_in_viewport(37.0), 9); // 37/4 = 9.25 -> floors to 9
}

// ============================================================================
// TIME SPACING TESTS
// ============================================================================

#[test]
fn test_time_unit_approx_seconds() {
    let spacing = TimeTickSpacing {
        unit: TimeUnit::Second,
        step: 1,
    };
    assert_eq!(spacing.approx_seconds(), 1.0);

    let spacing = TimeTickSpacing {
        unit: TimeUnit::Minute,
        step: 5,
    };
    assert_eq!(spacing.approx_seconds(), 300.0);

    let spacing = TimeTickSpacing {
        unit: TimeUnit::Hour,
        step: 1,
    };
    assert_eq!(spacing.approx_seconds(), 3600.0);
}

#[test]
fn test_choose_time_unit_for_various_spans() {
    // Seconds for very short spans
    assert_eq!(choose_time_unit(10.0), TimeUnit::Second);

    // Minutes for minute-scale spans
    assert_eq!(choose_time_unit(300.0), TimeUnit::Minute);

    // Hours for hour-scale spans
    assert_eq!(choose_time_unit(10800.0), TimeUnit::Hour);

    // Days for multi-day spans
    assert_eq!(choose_time_unit(259200.0), TimeUnit::Day);
}

#[test]
fn test_candidate_steps_for_each_unit() {
    assert_eq!(candidate_steps(TimeUnit::Second), &[1, 2, 5, 10, 15, 30]);
    assert_eq!(candidate_steps(TimeUnit::Minute), &[1, 2, 5, 10, 15, 30]);
    assert_eq!(candidate_steps(TimeUnit::Hour), &[1, 2, 3, 4, 6, 12]);
    assert_eq!(candidate_steps(TimeUnit::Day), &[1, 2, 3, 5, 7, 10, 14]);
    assert_eq!(candidate_steps(TimeUnit::Month), &[1, 2, 3, 6, 12]);
    assert_eq!(candidate_steps(TimeUnit::Year), &[1, 2, 5, 10, 20]);
}

#[test]
fn test_select_time_spacing_produces_reasonable_ticks() {
    // For a 1-hour span
    let spacing = select_time_spacing(3600.0, None);
    assert_eq!(spacing.unit, TimeUnit::Minute);

    // For a 1-day span
    let spacing = select_time_spacing(86400.0, None);
    assert_eq!(spacing.unit, TimeUnit::Hour);
}

#[test]
fn test_time_tick_spacing_generate_ticks_basic() {
    let spacing = TimeTickSpacing {
        unit: TimeUnit::Hour,
        step: 1,
    };

    // Generate ticks for a 6-hour span
    let start = 1_600_000_000.0; // Some arbitrary timestamp
    let end = start + 6.0 * 3600.0;
    let ticks = spacing.generate_ticks(start, end);

    // Should have at least 5 ticks (0, 1, 2, 3, 4, 5, 6 hours)
    assert!(ticks.len() >= 5);
    assert!(ticks.len() <= 10);
}

#[test]
fn test_time_tick_spacing_handles_empty_range() {
    let spacing = TimeTickSpacing {
        unit: TimeUnit::Minute,
        step: 1,
    };

    // Empty range
    let ticks = spacing.generate_ticks(100.0, 100.0);
    assert_eq!(ticks.len(), 0);

    // Reversed range
    let ticks = spacing.generate_ticks(200.0, 100.0);
    assert_eq!(ticks.len(), 0);
}

#[test]
fn test_time_tick_spacing_hysteresis() {
    let span = 3600.0; // 1 hour

    // First call - no previous spacing
    let spacing1 = select_time_spacing(span, None);

    // Second call with previous spacing - should maintain if still valid
    let spacing2 = select_time_spacing(span * 1.1, Some(spacing1));

    // Should maintain same unit due to hysteresis
    assert_eq!(spacing1.unit, spacing2.unit);
}

#[test]
fn test_time_tick_spacing_changes_unit_when_needed() {
    let spacing1 = select_time_spacing(3600.0, None); // 1 hour
    let spacing2 = select_time_spacing(86400.0 * 30.0, None); // 30 days

    // Units should be different for vastly different spans
    assert_ne!(spacing1.unit, spacing2.unit);
}

// ============================================================================
// PAN AND VIEWPORT CALCULATIONS
// ============================================================================

#[test]
fn test_pan_offset_calculations() {
    let zoom = ZoomX::default();
    let bar_width = zoom.bar_width_px as f64;
    let gap = 1.0;
    let bar_spacing = bar_width + gap;
    let seconds_per_bar = zoom.current_lod_level.seconds();

    // Pan by 100 pixels to the right
    let dx = 100.0;
    let bars_moved = dx / bar_spacing;
    let time_offset = bars_moved * seconds_per_bar;

    // With default 3px bars + 1px gap = 4px spacing
    // 100px / 4px = 25 bars
    // 25 bars * 60 seconds = 1500 seconds
    assert_eq!(bars_moved, 25.0);
    assert_eq!(time_offset, 1500.0);
}

#[test]
fn test_pan_offset_with_different_lod_levels() {
    let mut zoom = ZoomX::default();

    // Test with M1 (60s)
    zoom.current_lod_level = LodLevel::M1;
    let bar_spacing = (zoom.bar_width_px as f64) + 1.0;
    let dx = 40.0; // 10 bars worth
    let bars = dx / bar_spacing;
    let offset_m1 = bars * zoom.current_lod_level.seconds();

    // Test with M5 (300s)
    zoom.current_lod_level = LodLevel::M5;
    let offset_m5 = bars * zoom.current_lod_level.seconds();

    // M5 should have 5x the time offset for same number of bars
    assert_eq!(offset_m5 / offset_m1, 5.0);
}

#[test]
fn test_visible_range_calculations() {
    // Simulate viewport state visible range calculation
    let viewport_width = 800.0;
    let zoom = ZoomX::default();
    let num_bars = zoom.get_num_bars_in_viewport(viewport_width);

    let total_candles: usize = 1000;
    let pan_offset_seconds = 0.0; // At end
    let lod_seconds = zoom.current_lod_level.seconds();

    let bars_to_offset = (pan_offset_seconds / lod_seconds).round() as usize;
    let end_idx = total_candles.saturating_sub(bars_to_offset);
    let start_idx = end_idx.saturating_sub(num_bars as usize);

    // Should show last 200 bars (800px / 4px)
    assert_eq!(end_idx, 1000);
    assert_eq!(start_idx, 800);
}

#[test]
fn test_visible_range_with_pan() {
    let viewport_width = 800.0;
    let zoom = ZoomX::default();
    let num_bars = zoom.get_num_bars_in_viewport(viewport_width);

    let total_candles: usize = 1000;
    let pan_offset_seconds = 600.0; // 10 minutes back (10 bars at M1)
    let lod_seconds = zoom.current_lod_level.seconds();

    let bars_to_offset = (pan_offset_seconds / lod_seconds).round() as usize;
    let end_idx = total_candles.saturating_sub(bars_to_offset);
    let start_idx = end_idx.saturating_sub(num_bars as usize);

    // Should show bars from 790 to 990
    assert_eq!(end_idx, 990);
    assert_eq!(start_idx, 790);
}

#[test]
fn test_y_axis_pan_calculations() {
    let viewport_height = 600.0;
    let fixed_y_min = 100.0;
    let fixed_y_max = 200.0;
    let price_range = fixed_y_max - fixed_y_min;
    let price_per_pixel = price_range / viewport_height;

    // Pan down by 60 pixels (shows higher prices)
    let dy = 60.0;
    let pan_offset_y = dy * price_per_pixel;

    // Should pan by 10 price units (100 price range / 600 pixels * 60 pixels)
    assert_eq!(pan_offset_y, 10.0);
}

// ============================================================================
// DATA INTEGRITY TESTS
// ============================================================================

#[test]
fn test_lod_all_levels_are_unique() {
    let levels = LodLevel::all_levels();
    let mut seconds: Vec<f64> = levels.iter().map(|l| l.seconds()).collect();
    let original_len = seconds.len();

    seconds.sort_by(|a, b| a.partial_cmp(b).unwrap());
    seconds.dedup();

    // All levels should have unique second values
    assert_eq!(seconds.len(), original_len);
}

#[test]
fn test_lod_all_levels_are_sorted() {
    let levels = LodLevel::all_levels();
    let seconds: Vec<f64> = levels.iter().map(|l| l.seconds()).collect();

    // Verify they're in ascending order
    for i in 1..seconds.len() {
        assert!(
            seconds[i] > seconds[i - 1],
            "LOD levels not in ascending order at index {}",
            i
        );
    }
}

#[test]
fn test_lod_labels_are_unique() {
    let levels = LodLevel::all_levels();
    let labels: Vec<&str> = levels.iter().map(|l| l.label()).collect();
    let mut unique_labels = labels.clone();
    unique_labels.sort();
    unique_labels.dedup();

    assert_eq!(labels.len(), unique_labels.len());
}

// ============================================================================
// EDGE CASE TESTS
// ============================================================================

#[test]
fn test_zoom_at_boundaries() {
    let mut zoom = ZoomX::default();
    zoom.set_min_lod_from_interval(1);
    zoom.current_lod_level = LodLevel::S1; // Finest level

    // Try to zoom finer - should stay at S1
    zoom.handle_lod_change(1.0, LodLevel::all_levels());
    assert_eq!(zoom.current_lod_level, LodLevel::S1);

    // Zoom to coarsest level
    zoom.current_lod_level = LodLevel::Month1;
    zoom.handle_lod_change(-1.0, LodLevel::all_levels());
    assert_eq!(zoom.current_lod_level, LodLevel::Month1);
}

#[test]
fn test_viewport_with_zero_width() {
    let zoom = ZoomX::default();
    let num_bars = zoom.get_num_bars_in_viewport(0.0);
    assert_eq!(num_bars, 0);
}

#[test]
fn test_viewport_with_tiny_width() {
    let zoom = ZoomX::default();
    let num_bars = zoom.get_num_bars_in_viewport(1.0);
    // 1 pixel / 4 pixels per bar = 0.25 -> floors to 0
    assert_eq!(num_bars, 0);
}

#[test]
fn test_pan_offset_clamping() {
    let pan_offset: f64 = -100.0; // Negative offset (invalid)
    let clamped = pan_offset.max(0.0);
    assert_eq!(clamped, 0.0);
}

#[test]
fn test_saturating_sub_prevents_underflow() {
    let total_candles = 100usize;
    let num_bars = 200usize;

    // Without saturation, this would underflow
    let start_idx = total_candles.saturating_sub(num_bars);
    assert_eq!(start_idx, 0);
}

// ============================================================================
// INTEGRATION TESTS - Cross-component validation
// ============================================================================

#[test]
fn test_zoom_lod_consistency_with_time_spacing() {
    // Ensure LOD levels align with time spacing concepts
    let lod = LodLevel::M5;
    let seconds = lod.seconds();

    let spacing = TimeTickSpacing {
        unit: TimeUnit::Minute,
        step: 5,
    };

    assert_eq!(seconds, spacing.approx_seconds());
}

#[test]
fn test_zoom_sequence_maintains_order() {
    let mut zoom = ZoomX::default();
    zoom.current_lod_level = LodLevel::M1;
    zoom.set_min_lod_from_interval(1);

    let mut previous_seconds = zoom.current_lod_level.seconds();

    // Zoom out 5 times
    for _ in 0..5 {
        zoom.handle_lod_change(-1.0, LodLevel::all_levels());
        let current_seconds = zoom.current_lod_level.seconds();
        // Each level should have equal or more seconds
        assert!(current_seconds >= previous_seconds);
        previous_seconds = current_seconds;
    }
}

#[test]
fn test_lod_and_viewport_integration() {
    // Test that changing LOD level affects visible range correctly
    let viewport_width = 800.0;
    let mut zoom = ZoomX::default();

    // At M1 (60s), 800px shows 200 bars = 12000 seconds
    zoom.current_lod_level = LodLevel::M1;
    let num_bars_m1 = zoom.get_num_bars_in_viewport(viewport_width);
    let time_span_m1 = num_bars_m1 as f64 * zoom.current_lod_level.seconds();

    // At M5 (300s), 800px still shows 200 bars = 60000 seconds
    zoom.current_lod_level = LodLevel::M5;
    let num_bars_m5 = zoom.get_num_bars_in_viewport(viewport_width);
    let time_span_m5 = num_bars_m5 as f64 * zoom.current_lod_level.seconds();

    // Same number of bars but different time spans
    assert_eq!(num_bars_m1, num_bars_m5);
    assert_eq!(time_span_m5 / time_span_m1, 5.0);
}

#[test]
fn test_scroll_pan_consistency() {
    // Verify scroll and pan produce consistent time offsets
    let zoom = ZoomX::default();
    let viewport_width = 800.0;

    let bar_width = zoom.bar_width_px as f64;
    let gap = 1.0;
    let bar_spacing = bar_width + gap;
    let num_bars_visible = viewport_width / bar_spacing;
    let seconds_per_bar = zoom.current_lod_level.seconds();

    // Scroll by 10% of visible range
    let scroll_factor = num_bars_visible * 0.1 * seconds_per_bar;

    // Pan by equivalent pixels
    let pan_pixels = viewport_width * 0.1;
    let pan_bars = pan_pixels / bar_spacing;
    let pan_offset = pan_bars * seconds_per_bar;

    // Both should produce same time offset
    assert_eq!(scroll_factor, pan_offset);
}
