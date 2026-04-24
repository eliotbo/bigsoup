// Import all code from the example
include!("../examples/compress_test.rs");

// Helper to create NYC local timestamp (returns UTC timestamp)
// Note: New_York is imported from the included example file
use chrono::Timelike;
fn nyc_ts(y: i32, m: u32, d: u32, h: u32, min: u32, s: u32) -> i64 {
    New_York
        .with_ymd_and_hms(y, m, d, h, min, s)
        .single()
        .unwrap()
        .timestamp()
}

fn pixels_per_second_for_full_view(map: &CalendarMap, viewport_width: f64) -> f64 {
    let axis_len = map.axis_len() as f64;
    if axis_len <= 0.0 {
        1.0
    } else {
        viewport_width / axis_len
    }
}

#[test]
fn test_weekend_filter_arbitrary_timestamps() {
    // Sep 11-17, 2024: Wed-Tue (13 and 14 are Sat/Sun)
    let rules = Rules::default(); // drop_weekends: true, 8am-8pm

    // Create timestamps for Sep 11-17 at noon NYC time
    let timestamps = vec![
        nyc_ts(2024, 9, 11, 12, 0, 0), // Wed
        nyc_ts(2024, 9, 12, 12, 0, 0), // Thu
        nyc_ts(2024, 9, 13, 12, 0, 0), // Fri
        nyc_ts(2024, 9, 14, 12, 0, 0), // Sat - should be dropped
        nyc_ts(2024, 9, 15, 12, 0, 0), // Sun - should be dropped
        nyc_ts(2024, 9, 16, 12, 0, 0), // Mon
        nyc_ts(2024, 9, 17, 12, 0, 0), // Tue
    ];

    let start = timestamps[0] - 86400; // day before
    let end = timestamps[6] + 86400; // day after
    let map = build_calendar_map_range(start, end, &rules);

    // Weekend timestamps should map to None
    assert!(
        map.real_to_axis(timestamps[3]).is_none(),
        "Saturday should not map"
    );
    assert!(
        map.real_to_axis(timestamps[4]).is_none(),
        "Sunday should not map"
    );

    // Weekday timestamps should map to Some
    assert!(
        map.real_to_axis(timestamps[0]).is_some(),
        "Wednesday should map"
    );
    assert!(
        map.real_to_axis(timestamps[1]).is_some(),
        "Thursday should map"
    );
    assert!(
        map.real_to_axis(timestamps[2]).is_some(),
        "Friday should map"
    );
    assert!(
        map.real_to_axis(timestamps[5]).is_some(),
        "Monday should map"
    );
    assert!(
        map.real_to_axis(timestamps[6]).is_some(),
        "Tuesday should map"
    );
}

#[test]
fn test_night_filter_arbitrary_timestamps() {
    // Single day with various hours (8pm-8am should be compressed)
    let rules = Rules {
        tz: New_York,
        drop_weekends: false,
        day_start_h: 8,
        day_end_h: 20,
    };

    // Sep 15, 2024 (Sunday, but weekends not dropped)
    let timestamps = vec![
        nyc_ts(2024, 9, 15, 14, 0, 0), // 2pm - included
        nyc_ts(2024, 9, 15, 16, 0, 0), // 4pm - included
        nyc_ts(2024, 9, 15, 18, 0, 0), // 6pm - included
        nyc_ts(2024, 9, 15, 20, 0, 0), // 8pm - excluded (at boundary)
        nyc_ts(2024, 9, 15, 22, 0, 0), // 10pm - excluded
        nyc_ts(2024, 9, 16, 0, 0, 0),  // midnight - excluded
        nyc_ts(2024, 9, 16, 2, 0, 0),  // 2am - excluded
        nyc_ts(2024, 9, 16, 8, 0, 0),  // 8am - included (at boundary)
    ];

    let start = nyc_ts(2024, 9, 15, 0, 0, 0);
    let end = nyc_ts(2024, 9, 17, 0, 0, 0);
    let map = build_calendar_map_range(start, end, &rules);

    // Day hours should map
    assert!(map.real_to_axis(timestamps[0]).is_some(), "2pm should map");
    assert!(map.real_to_axis(timestamps[1]).is_some(), "4pm should map");
    assert!(map.real_to_axis(timestamps[2]).is_some(), "6pm should map");

    // Night hours should not map
    assert!(
        map.real_to_axis(timestamps[3]).is_none(),
        "8pm should not map"
    );
    assert!(
        map.real_to_axis(timestamps[4]).is_none(),
        "10pm should not map"
    );
    assert!(
        map.real_to_axis(timestamps[5]).is_none(),
        "midnight should not map"
    );
    assert!(
        map.real_to_axis(timestamps[6]).is_none(),
        "2am should not map"
    );

    // 8am should map (day_start_h is inclusive)
    assert!(map.real_to_axis(timestamps[7]).is_some(), "8am should map");
}

#[test]
fn test_regular_grid_weekend_compression() {
    // Create a regular 1-day grid over a week spanning a weekend
    let rules = Rules::default();

    let start = nyc_ts(2024, 9, 13, 8, 0, 0); // Fri 8am
    let end = nyc_ts(2024, 9, 17, 20, 0, 0); // Tue 8pm

    let map = build_calendar_map_range(start, end, &rules);

    // Generate regular grid: every 24 hours starting from Fri noon
    let grid_start = nyc_ts(2024, 9, 13, 12, 0, 0); // Fri noon
    let grid_times: Vec<i64> = (0..5).map(|i| grid_start + i * 86400).collect();

    let axis_positions: Vec<Option<i64>> =
        grid_times.iter().map(|&t| map.real_to_axis(t)).collect();

    // Fri and Mon should map, Sat/Sun should not
    assert!(axis_positions[0].is_some(), "Friday should map");
    assert!(axis_positions[1].is_none(), "Saturday should not map");
    assert!(axis_positions[2].is_none(), "Sunday should not map");
    assert!(axis_positions[3].is_some(), "Monday should map");
    assert!(axis_positions[4].is_some(), "Tuesday should map");
}

#[test]
fn test_regular_grid_night_compression() {
    // Create a regular 2-hour grid over a day/night boundary
    let rules = Rules {
        tz: New_York,
        drop_weekends: false,
        day_start_h: 8,
        day_end_h: 20,
    };

    let start = nyc_ts(2024, 9, 15, 8, 0, 0); // 8am
    let end = nyc_ts(2024, 9, 16, 20, 0, 0); // next day 8pm

    let map = build_calendar_map_range(start, end, &rules);

    // Generate 2-hour grid starting from 2pm
    let grid_start = nyc_ts(2024, 9, 15, 14, 0, 0); // 2pm
    let grid_times: Vec<i64> = (0..13)
        .map(|i| grid_start + i * 7200) // 2 hours = 7200 seconds
        .collect();

    let axis_positions: Vec<Option<i64>> =
        grid_times.iter().map(|&t| map.real_to_axis(t)).collect();

    // Expected pattern: day hours map, night hours (8pm-8am) don't
    // 2pm, 4pm, 6pm should map
    assert!(axis_positions[0].is_some(), "2pm should map");
    assert!(axis_positions[1].is_some(), "4pm should map");
    assert!(axis_positions[2].is_some(), "6pm should map");

    // 8pm, 10pm, midnight, 2am, 4am, 6am should not map
    assert!(axis_positions[3].is_none(), "8pm should not map");
    assert!(axis_positions[4].is_none(), "10pm should not map");
    assert!(axis_positions[5].is_none(), "midnight should not map");
    assert!(axis_positions[6].is_none(), "2am should not map");
    assert!(axis_positions[7].is_none(), "4am should not map");
    assert!(axis_positions[8].is_none(), "6am should not map");

    // 8am, 10am, noon, 2pm should map
    assert!(axis_positions[9].is_some(), "8am should map");
    assert!(axis_positions[10].is_some(), "10am should map");
    assert!(axis_positions[11].is_some(), "noon should map");
    assert!(axis_positions[12].is_some(), "2pm next day should map");
}

#[test]
fn test_round_trip_conversion() {
    let rules = Rules::default();
    let start = nyc_ts(2024, 9, 11, 9, 0, 0);
    let end = nyc_ts(2024, 9, 18, 19, 0, 0);

    let map = build_calendar_map_range(start, end, &rules);

    // Test round-trip for various weekday timestamps
    let test_times = vec![
        nyc_ts(2024, 9, 11, 10, 0, 0),  // Wed 10am
        nyc_ts(2024, 9, 12, 15, 30, 0), // Thu 3:30pm
        nyc_ts(2024, 9, 13, 19, 0, 0),  // Fri 7pm
        nyc_ts(2024, 9, 16, 8, 0, 0),   // Mon 8am
        nyc_ts(2024, 9, 17, 12, 0, 0),  // Tue noon
    ];

    for &t in &test_times {
        if let Some(axis_pos) = map.real_to_axis(t) {
            let recovered = map.axis_to_real(axis_pos).unwrap();
            assert_eq!(recovered, t, "Round-trip failed for timestamp {}", t);
        }
    }
}

#[test]
fn test_axis_positions_are_contiguous() {
    let rules = Rules::default();
    let start = nyc_ts(2024, 9, 11, 9, 0, 0);
    let end = nyc_ts(2024, 9, 18, 19, 0, 0);

    let map = build_calendar_map_range(start, end, &rules);

    // Create timestamps every hour during weekdays
    let mut prev_axis: Option<i64> = None;

    for day in 11..=17 {
        let is_weekend = day == 14 || day == 15; // Sat/Sun
        if is_weekend {
            continue;
        }

        for hour in 8..20 {
            let t = nyc_ts(2024, 9, day, hour, 0, 0);
            if let Some(axis_pos) = map.real_to_axis(t) {
                if let Some(prev) = prev_axis {
                    // Each hour should be 3600 seconds apart on the axis
                    assert_eq!(
                        axis_pos - prev,
                        3600,
                        "Axis not contiguous at day {} hour {}",
                        day,
                        hour
                    );
                }
                prev_axis = Some(axis_pos);
            }
        }
    }
}

#[test]
fn test_empty_range() {
    let rules = Rules::default();
    let t = nyc_ts(2024, 9, 15, 12, 0, 0);
    let map = build_calendar_map_range(t, t, &rules);

    assert_eq!(map.spans.len(), 0, "Empty range should produce no spans");
    assert_eq!(
        map.axis_len(),
        0,
        "Empty range should have zero axis length"
    );
}

#[test]
fn test_weekend_only_range() {
    let rules = Rules::default();

    // Sep 14-15, 2024 (Saturday-Sunday)
    let start = nyc_ts(2024, 9, 14, 0, 0, 0);
    let end = nyc_ts(2024, 9, 16, 0, 0, 0);

    let map = build_calendar_map_range(start, end, &rules);

    assert_eq!(
        map.spans.len(),
        0,
        "Weekend-only range should produce no spans"
    );
    assert_eq!(
        map.axis_len(),
        0,
        "Weekend-only range should have zero axis length"
    );
}

#[test]
fn test_night_only_range() {
    let rules = Rules {
        tz: New_York,
        drop_weekends: false,
        day_start_h: 8,
        day_end_h: 20,
    };

    // Sep 15, 2024 from 8pm to next day 8am (12 hours of night)
    let start = nyc_ts(2024, 9, 15, 20, 0, 0);
    let end = nyc_ts(2024, 9, 16, 8, 0, 0);

    let map = build_calendar_map_range(start, end, &rules);

    assert_eq!(
        map.spans.len(),
        0,
        "Night-only range should produce no spans"
    );
    assert_eq!(
        map.axis_len(),
        0,
        "Night-only range should have zero axis length"
    );
}

#[test]
fn test_no_filters() {
    // No weekend filtering, but we still need valid hours (0-23, not 24)
    // This test verifies that weekends are included when drop_weekends=false
    let rules = Rules {
        tz: New_York,
        drop_weekends: false,
        day_start_h: 0,
        day_end_h: 23, // Valid hour range is 0-23
    };

    // Sep 14-15 is a weekend (Sat-Sun)
    let start = nyc_ts(2024, 9, 14, 0, 0, 0); // Saturday
    let end = nyc_ts(2024, 9, 16, 0, 0, 0); // Monday

    let map = build_calendar_map_range(start, end, &rules);

    // With drop_weekends=false, weekend days should be included
    // We should have spans for both Saturday and Sunday (0-23 hours each = 23*3600)
    assert!(map.spans.len() >= 2, "Should have spans for weekend days");

    // Saturday noon should map (since weekends not dropped)
    let sat_noon = nyc_ts(2024, 9, 14, 12, 0, 0);
    assert!(
        map.real_to_axis(sat_noon).is_some(),
        "Saturday should map when drop_weekends=false"
    );

    // Sunday noon should map
    let sun_noon = nyc_ts(2024, 9, 15, 12, 0, 0);
    assert!(
        map.real_to_axis(sun_noon).is_some(),
        "Sunday should map when drop_weekends=false"
    );
}

#[test]
fn test_axis_len_calculation() {
    let rules = Rules::default(); // 8am-8pm, weekends dropped

    // Sep 11-13 (Wed-Fri, 3 weekdays)
    let start = nyc_ts(2024, 9, 11, 8, 0, 0);
    let end = nyc_ts(2024, 9, 13, 20, 0, 0);

    let map = build_calendar_map_range(start, end, &rules);

    // Each day has 12 hours (8am-8pm) = 43200 seconds
    // 3 days = 129600 seconds
    let expected_len = 3 * 12 * 3600;
    assert_eq!(
        map.axis_len(),
        expected_len,
        "Axis length should equal sum of trading hours"
    );
}

#[test]
fn test_span_boundaries() {
    let rules = Rules::default();

    let start = nyc_ts(2024, 9, 13, 8, 0, 0); // Fri 8am
    let end = nyc_ts(2024, 9, 16, 20, 0, 0); // Mon 8pm

    let map = build_calendar_map_range(start, end, &rules);

    // Should have 2 spans: Friday and Monday
    assert_eq!(map.spans.len(), 2, "Should have 2 spans (Fri and Mon)");

    // Check Friday span
    assert_eq!(map.spans[0].real_start, nyc_ts(2024, 9, 13, 8, 0, 0));
    assert_eq!(map.spans[0].real_end, nyc_ts(2024, 9, 13, 20, 0, 0));
    assert_eq!(map.spans[0].axis_start, 0);

    // Check Monday span
    assert_eq!(map.spans[1].real_start, nyc_ts(2024, 9, 16, 8, 0, 0));
    assert_eq!(map.spans[1].real_end, nyc_ts(2024, 9, 16, 20, 0, 0));
    assert_eq!(map.spans[1].axis_start, 12 * 3600); // After Friday's 12 hours
}

#[test]
fn test_partial_day_at_boundaries() {
    let rules = Rules::default();

    // Start mid-day Friday, end mid-day Monday
    let start = nyc_ts(2024, 9, 13, 14, 0, 0); // Fri 2pm
    let end = nyc_ts(2024, 9, 16, 14, 0, 0); // Mon 2pm

    let map = build_calendar_map_range(start, end, &rules);

    // Fri: 2pm-8pm = 6 hours
    // Mon: 8am-2pm = 6 hours
    let expected_len = 2 * 6 * 3600;
    assert_eq!(
        map.axis_len(),
        expected_len,
        "Should handle partial days correctly"
    );
}

#[test]
fn test_multiple_weeks() {
    let rules = Rules::default();

    // 3 weeks: Sep 9 - Sep 27 (Mon-Fri)
    let start = nyc_ts(2024, 9, 9, 8, 0, 0);
    let end = nyc_ts(2024, 9, 27, 20, 0, 0);

    let map = build_calendar_map_range(start, end, &rules);

    // 3 weeks = ~15 weekdays (accounting for boundaries)
    // Each weekday = 12 hours
    let weekdays = map.spans.len();
    assert!(
        weekdays >= 13 && weekdays <= 15,
        "Should have approximately 13-15 weekdays in 3 weeks"
    );
}

#[test]
fn test_binary_search_edge_cases() {
    let rules = Rules::default();

    let start = nyc_ts(2024, 9, 11, 8, 0, 0);
    let end = nyc_ts(2024, 9, 13, 20, 0, 0);

    let map = build_calendar_map_range(start, end, &rules);

    // Test exact boundaries
    assert!(
        map.real_to_axis(nyc_ts(2024, 9, 11, 8, 0, 0)).is_some(),
        "Start boundary should map"
    );
    assert!(
        map.real_to_axis(nyc_ts(2024, 9, 11, 20, 0, 0)).is_none(),
        "End boundary (exclusive) should not map"
    );

    // Test before and after range
    assert!(
        map.real_to_axis(nyc_ts(2024, 9, 11, 7, 59, 59)).is_none(),
        "Before range should not map"
    );
    assert!(
        map.real_to_axis(nyc_ts(2024, 9, 13, 20, 0, 1)).is_none(),
        "After range should not map"
    );
}

#[test]
fn test_compress_sequence_weekend_example() {
    // This matches the example from the doc comment:
    // 11 Sep | 12 Sep | 13 Sep | 14 Sep | 15 Sep →
    // 11 Sep | 12 Sep | 15 Sep | 16 Sep | 17 Sep (13 and 14 were weekend days)
    let rules = Rules::default();

    // Sep 11-17, 2024: Wed-Tue (13 and 14 are Sat/Sun)
    let start = nyc_ts(2024, 9, 11, 8, 0, 0);
    let end = nyc_ts(2024, 9, 18, 20, 0, 0);
    let map = build_calendar_map_range(start, end, &rules);

    // Input timestamps at noon each day
    let input_timestamps = vec![
        nyc_ts(2024, 9, 11, 12, 0, 0), // Wed
        nyc_ts(2024, 9, 12, 12, 0, 0), // Thu
        nyc_ts(2024, 9, 13, 12, 0, 0), // Fri
        nyc_ts(2024, 9, 14, 12, 0, 0), // Sat - weekend
        nyc_ts(2024, 9, 15, 12, 0, 0), // Sun - weekend
        nyc_ts(2024, 9, 16, 12, 0, 0), // Mon
        nyc_ts(2024, 9, 17, 12, 0, 0), // Tue
    ];

    let compressed = compress_sequence(&input_timestamps, &map);

    // Weekdays should map, weekends should be None
    assert!(compressed[0].is_some(), "Wed should map");
    assert!(compressed[1].is_some(), "Thu should map");
    assert!(compressed[2].is_some(), "Fri should map");
    assert!(compressed[3].is_none(), "Sat should not map");
    assert!(compressed[4].is_none(), "Sun should not map");
    assert!(compressed[5].is_some(), "Mon should map");
    assert!(compressed[6].is_some(), "Tue should map");

    // Collect only the valid (weekday) axis positions
    let valid_axis_positions: Vec<i64> = compressed.iter().filter_map(|&x| x).collect();

    // These should be contiguous and increasing
    assert_eq!(valid_axis_positions.len(), 5, "Should have 5 weekdays");
    for i in 1..valid_axis_positions.len() {
        assert!(
            valid_axis_positions[i] > valid_axis_positions[i - 1],
            "Axis positions should be increasing"
        );
    }
}

#[test]
fn test_compress_sequence_night_example() {
    // This matches the night filter example from the doc comment:
    // 14:00 | 16:00 | 18:00 | 20:00 | 22:00 | 00:00 | 2:00
    // → 14:00 | 16:00 | 18:00 | 20:00 | 8:00 | 10:00 | 12:00
    let rules = Rules {
        tz: New_York,
        drop_weekends: false,
        day_start_h: 8,
        day_end_h: 20,
    };

    let start = nyc_ts(2024, 9, 15, 8, 0, 0);
    let end = nyc_ts(2024, 9, 16, 20, 0, 0);
    let map = build_calendar_map_range(start, end, &rules);

    // Input timestamps matching the example
    let input_timestamps = vec![
        nyc_ts(2024, 9, 15, 14, 0, 0), // 2pm - day
        nyc_ts(2024, 9, 15, 16, 0, 0), // 4pm - day
        nyc_ts(2024, 9, 15, 18, 0, 0), // 6pm - day
        nyc_ts(2024, 9, 15, 20, 0, 0), // 8pm - night
        nyc_ts(2024, 9, 15, 22, 0, 0), // 10pm - night
        nyc_ts(2024, 9, 16, 0, 0, 0),  // midnight - night
        nyc_ts(2024, 9, 16, 2, 0, 0),  // 2am - night
    ];

    let compressed = compress_sequence(&input_timestamps, &map);

    // Day hours should map, night hours should be None
    assert!(compressed[0].is_some(), "2pm should map");
    assert!(compressed[1].is_some(), "4pm should map");
    assert!(compressed[2].is_some(), "6pm should map");
    assert!(compressed[3].is_none(), "8pm should not map");
    assert!(compressed[4].is_none(), "10pm should not map");
    assert!(compressed[5].is_none(), "midnight should not map");
    assert!(compressed[6].is_none(), "2am should not map");

    // The compressed positions should be 2 hours apart (7200 seconds)
    let valid_positions: Vec<i64> = compressed.iter().filter_map(|&x| x).collect();

    assert_eq!(valid_positions.len(), 3, "Should have 3 day-time entries");
    assert_eq!(
        valid_positions[1] - valid_positions[0],
        7200,
        "2 hours apart"
    );
    assert_eq!(
        valid_positions[2] - valid_positions[1],
        7200,
        "2 hours apart"
    );
}

#[test]
fn test_decompress_sequence() {
    let rules = Rules::default();
    let start = nyc_ts(2024, 9, 11, 8, 0, 0);
    let end = nyc_ts(2024, 9, 13, 20, 0, 0);
    let map = build_calendar_map_range(start, end, &rules);

    // Create some axis positions (every 4 hours on compressed axis)
    let axis_positions: Vec<i64> = (0..6)
        .map(|i| i * 14400) // 4 hours = 14400 seconds
        .collect();

    let decompressed = decompress_sequence(&axis_positions, &map);

    // All should successfully decompress
    assert!(
        decompressed.iter().all(|x| x.is_some()),
        "All axis positions should decompress"
    );

    // Verify round-trip
    let recompressed = compress_sequence(
        &decompressed.iter().filter_map(|&x| x).collect::<Vec<_>>(),
        &map,
    );

    for (i, &orig_axis) in axis_positions.iter().enumerate() {
        assert_eq!(
            recompressed[i],
            Some(orig_axis),
            "Round-trip should preserve axis position"
        );
    }
}

#[test]
fn test_compress_sequence_empty() {
    let rules = Rules::default();
    let start = nyc_ts(2024, 9, 11, 8, 0, 0);
    let end = nyc_ts(2024, 9, 13, 20, 0, 0);
    let map = build_calendar_map_range(start, end, &rules);

    let empty: Vec<i64> = vec![];
    let compressed = compress_sequence(&empty, &map);
    assert_eq!(
        compressed.len(),
        0,
        "Empty input should produce empty output"
    );
}

#[test]
fn test_compress_sequence_all_filtered() {
    let rules = Rules::default();
    let start = nyc_ts(2024, 9, 11, 8, 0, 0);
    let end = nyc_ts(2024, 9, 18, 20, 0, 0);
    let map = build_calendar_map_range(start, end, &rules);

    // All weekend timestamps
    let weekend_timestamps = vec![
        nyc_ts(2024, 9, 14, 12, 0, 0), // Sat
        nyc_ts(2024, 9, 14, 16, 0, 0), // Sat
        nyc_ts(2024, 9, 15, 12, 0, 0), // Sun
        nyc_ts(2024, 9, 15, 16, 0, 0), // Sun
    ];

    let compressed = compress_sequence(&weekend_timestamps, &map);

    assert!(
        compressed.iter().all(|x| x.is_none()),
        "All weekend timestamps should be filtered out"
    );
}

#[test]
fn test_compress_decompress_regular_grid() {
    let rules = Rules::default();
    let start = nyc_ts(2024, 9, 11, 8, 0, 0);
    let end = nyc_ts(2024, 9, 18, 20, 0, 0);
    let map = build_calendar_map_range(start, end, &rules);

    // Create a regular grid every hour for a week
    let mut timestamps = Vec::new();
    let mut t = nyc_ts(2024, 9, 11, 8, 0, 0);
    for _ in 0..(7 * 24) {
        timestamps.push(t);
        t += 3600; // 1 hour
    }

    let compressed = compress_sequence(&timestamps, &map);

    // Count how many mapped
    let mapped_count = compressed.iter().filter(|x| x.is_some()).count();

    // Should be 5 weekdays * 12 hours/day = 60 hours
    assert_eq!(
        mapped_count, 60,
        "Should have 60 trading hours in a work week"
    );

    // Verify the compressed positions are contiguous
    let valid_positions: Vec<i64> = compressed.iter().filter_map(|&x| x).collect();

    for i in 1..valid_positions.len() {
        assert_eq!(
            valid_positions[i] - valid_positions[i - 1],
            3600,
            "Compressed positions should be 1 hour apart"
        );
    }
}

#[test]
fn test_compressed_plot_ticks_weekend_filter() {
    // Demonstrate the doc comment example:
    // 11 Sep | 12 Sep | 13 Sep | 14 Sep | 15 Sep →
    // 11 Sep | 12 Sep | 15 Sep | 16 Sep | 17 Sep
    let rules = Rules::default(); // weekends dropped, 8am-8pm

    // Sep 11-17, 2024 (Wed-Tue, includes Sat/Sun)
    let start = nyc_ts(2025, 9, 11, 8, 0, 0); // Wed 8am
    let end = nyc_ts(2025, 9, 17, 20, 0, 0); // Tue 8pm
    let map = build_calendar_map_range(start, end, &rules);

    // Verify the map spans 5 weekdays (Wed, Thu, Fri, Mon, Tue)
    assert_eq!(map.spans.len(), 5, "Should have 5 weekday spans");
    assert_eq!(
        map.axis_len(),
        60 * 3600,
        "Should cover 60 trading hours (5 days × 12 hours)"
    );

    let pps = pixels_per_second_for_full_view(&map, 800.0);
    let ticks = generate_compressed_plot_ticks(800.0, pps, &map)
        .expect("ticks should be generated for visible map range");

    assert!(
        (4..=10).contains(&ticks.axis_positions.len()),
        "Expected a manageable number of major grid lines, got {}",
        ticks.axis_positions.len()
    );

    // Verify ticks are in compressed space and increasing
    for window in ticks.axis_positions.windows(2) {
        assert!(window[1] > window[0], "Ticks should be increasing");
    }

    for axis_pos in &ticks.axis_positions {
        let utc = map
            .axis_to_real(*axis_pos)
            .expect("tick should map to real time");
        let dt = New_York
            .timestamp_opt(utc, 0)
            .single()
            .expect("valid timestamp");
        assert_eq!(dt.minute(), 0, "Grid lines should land on whole hours");
        assert_eq!(dt.second(), 0, "Grid lines should land on whole hours");
    }

    // Explicitly verify the weekend collapse: 11 Sep | 12 Sep | 13 Sep | 16 Sep | 17 Sep
    // Check that ticks map to the correct dates (Sept 14-15 weekend filtered out)
    use std::collections::HashSet;
    let dates_with_ticks: HashSet<(u32, u32)> = ticks
        .axis_positions
        .iter()
        .filter_map(|&axis_pos| map.axis_to_real(axis_pos))
        .map(|utc_ts| {
            let dt = New_York.timestamp_opt(utc_ts, 0).single().unwrap();
            (dt.month(), dt.day())
        })
        .collect();

    let mut sorted_dates: Vec<_> = dates_with_ticks.into_iter().collect();
    sorted_dates.sort();

    use chrono::Weekday::{Sat, Sun};
    for (month, day) in &sorted_dates {
        let dt = New_York
            .with_ymd_and_hms(2025, *month, *day, 8, 0, 0)
            .earliest()
            .unwrap();
        assert!(
            !matches!(dt.weekday(), Sat | Sun),
            "Weekend dates should be filtered out, found {:?}",
            dt.weekday()
        );
    }

    assert!(
        sorted_dates.len() >= 4,
        "Expected ticks to cover multiple weekdays, got {:?}",
        sorted_dates
    );
}

#[test]
fn test_compressed_plot_ticks_night_filter() {
    // Demonstrate the night filter example:
    // 14:00 | 16:00 | 18:00 | 20:00 | 22:00 | 00:00 | 2:00
    // → 14:00 | 16:00 | 18:00 | (filtered) | (filtered) | (filtered) | (filtered)
    // Then next day: 8:00 | 10:00 | 12:00 | 14:00
    let rules = Rules {
        tz: New_York,
        drop_weekends: false,
        day_start_h: 8,
        day_end_h: 20,
    };

    let start = nyc_ts(2024, 9, 15, 14, 0, 0); // 2pm
    let end = nyc_ts(2024, 9, 16, 14, 0, 0); // next day 2pm
    let map = build_calendar_map_range(start, end, &rules);

    // Generate ticks with 1-hour interval
    let pps = pixels_per_second_for_full_view(&map, 800.0);
    let ticks = generate_compressed_plot_ticks(800.0, pps, &map)
        .expect("ticks should be generated for intra-day range");

    assert!(
        (4..=10).contains(&ticks.axis_positions.len()),
        "Expected compact intra-day ticks across the visible range, got {}",
        ticks.axis_positions.len()
    );

    for window in ticks.axis_positions.windows(2) {
        assert!(window[1] > window[0], "Ticks must remain ordered");
    }

    for axis_pos in &ticks.axis_positions {
        let utc = map
            .axis_to_real(*axis_pos)
            .expect("tick should map to real time");
        let dt = New_York
            .timestamp_opt(utc, 0)
            .single()
            .expect("valid timestamp");
        assert_eq!(dt.minute(), 0, "Intra-day grid should align to whole hours");
        assert_eq!(dt.second(), 0, "Intra-day grid should align to whole hours");
    }
}

#[test]
fn test_compressed_plot_ticks_hourly_with_weekends() {
    let rules = Rules::default();

    // Full week including weekend
    let start = nyc_ts(2024, 9, 13, 8, 0, 0); // Fri 8am
    let end = nyc_ts(2024, 9, 16, 20, 0, 0); // Mon 8pm
    let map = build_calendar_map_range(start, end, &rules);

    // Generate hourly ticks
    let pps = pixels_per_second_for_full_view(&map, 800.0);
    let ticks = generate_compressed_plot_ticks(800.0, pps, &map)
        .expect("ticks should be generated around weekend gap");

    assert!(
        (3..=8).contains(&ticks.axis_positions.len()),
        "Expected a small number of hourly anchors around the weekend gap, got {}",
        ticks.axis_positions.len()
    );

    for window in ticks.axis_positions.windows(2) {
        assert!(
            window[1] > window[0],
            "Ticks must stay ordered across the weekend"
        );
    }

    for axis_pos in &ticks.axis_positions {
        let utc = map
            .axis_to_real(*axis_pos)
            .expect("tick should map to real time");
        let dt = New_York
            .timestamp_opt(utc, 0)
            .single()
            .expect("valid timestamp");
        assert_eq!(
            dt.minute(),
            0,
            "Weekend-aware grid should land on whole hours"
        );
        assert_eq!(
            dt.second(),
            0,
            "Weekend-aware grid should land on whole hours"
        );
    }
}

#[test]
fn test_compressed_plot_ticks_5min_granularity() {
    let rules = Rules::default();

    let start = nyc_ts(2024, 9, 11, 9, 0, 0); // Wed 9am
    let end = nyc_ts(2024, 9, 11, 11, 0, 0); // Wed 11am
    let map = build_calendar_map_range(start, end, &rules);

    // Generate 5-minute ticks
    let pps = pixels_per_second_for_full_view(&map, 800.0);
    let ticks = generate_compressed_plot_ticks(800.0, pps, &map)
        .expect("ticks should be generated within trading window");

    assert!(
        !ticks.axis_positions.is_empty(),
        "Expected at least one grid line within the 2h trading window"
    );

    for window in ticks.axis_positions.windows(2) {
        assert!(
            window[1] > window[0],
            "Ticks should be ordered for 5-minute slices"
        );
    }

    for axis_pos in &ticks.axis_positions {
        let utc = map
            .axis_to_real(*axis_pos)
            .expect("tick should map to real time");
        let dt = New_York
            .timestamp_opt(utc, 0)
            .single()
            .expect("valid timestamp");
        assert_eq!(
            dt.minute() % 5,
            0,
            "Grid lines should fall on 5-minute boundaries"
        );
        assert_eq!(
            dt.second(),
            0,
            "Grid lines should fall on 5-minute boundaries"
        );
    }
}

#[test]
fn test_compressed_plot_ticks_crosses_midnight() {
    let rules = Rules {
        tz: New_York,
        drop_weekends: false,
        day_start_h: 8,
        day_end_h: 20,
    };

    // From late one day to early next day
    let start = nyc_ts(2024, 9, 15, 18, 0, 0); // 6pm
    let end = nyc_ts(2024, 9, 16, 10, 0, 0); // 10am next day
    let map = build_calendar_map_range(start, end, &rules);

    // Generate hourly ticks
    let pps = pixels_per_second_for_full_view(&map, 800.0);
    let ticks = generate_compressed_plot_ticks(800.0, pps, &map)
        .expect("ticks should be generated across night gap");

    // Day 1: 18, 19 = 2 ticks (20 is at boundary, filtered)
    // Night: filtered
    // Day 2: 8, 9, 10 = 3 ticks
    // Total: 4 or 5 ticks depending on alignment
    assert!(
        ticks.axis_positions.len() >= 4 && ticks.axis_positions.len() <= 5,
        "Should have 4-5 ticks across the night gap, got {}",
        ticks.axis_positions.len()
    );

    // All ticks should be 1 hour apart in compressed space
    for window in ticks.axis_positions.windows(2) {
        assert_eq!(
            window[1] - window[0],
            3600,
            "All ticks should be 1 hour apart even across night gap"
        );
    }
}

#[test]
fn test_compressed_plot_ticks_empty_range() {
    let rules = Rules::default();

    let start = nyc_ts(2024, 9, 14, 12, 0, 0); // Sat noon
    let end = nyc_ts(2024, 9, 14, 16, 0, 0); // Sat 4pm
    let map = build_calendar_map_range(start, end, &rules);

    // Generate ticks in weekend range (all filtered)
    let pps = pixels_per_second_for_full_view(&map, 800.0);
    let ticks = generate_compressed_plot_ticks(800.0, pps, &map);

    assert!(
        ticks.is_none(),
        "Should have no ticks in weekend-only range"
    );
}

#[test]
fn test_compressed_plot_ticks_alignment() {
    let rules = Rules::default();

    // Start at an odd time (not aligned to hour)
    let start = nyc_ts(2024, 9, 11, 9, 23, 0); // Wed 9:23am
    let end = nyc_ts(2024, 9, 11, 12, 47, 0); // Wed 12:47pm
    let map = build_calendar_map_range(start, end, &rules);

    // Generate hourly ticks
    let pps = pixels_per_second_for_full_view(&map, 800.0);
    let ticks =
        generate_compressed_plot_ticks(800.0, pps, &map).expect("hourly ticks should be generated");

    // Should align to hour boundaries: 10:00, 11:00, 12:00
    assert_eq!(
        ticks.axis_positions.len(),
        3,
        "Should have 3 aligned hourly ticks"
    );

    // Decompress first tick and verify it's at a round hour
    let first_tick_time = map.axis_to_real(ticks.axis_positions[0]).unwrap();
    let dt = chrono::DateTime::from_timestamp(first_tick_time, 0)
        .unwrap()
        .with_timezone(&New_York);

    assert_eq!(
        dt.minute(),
        0,
        "First tick should be aligned to hour boundary"
    );
    assert_eq!(
        dt.second(),
        0,
        "First tick should be aligned to hour boundary"
    );
}
