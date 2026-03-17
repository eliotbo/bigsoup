//! Generate synthetic LOB data and render a depth timeline screenshot.
//! Usage: cargo run --bin depth_timeline_screenshot

use vizza::config::{ColorPalette, Theme};
use vizza::depth_timeline::{DepthTimeline, DepthTimelineEntry, DepthTimelineState};
use vizza::depth_timeline_renderer::DepthTimelineRenderer;

fn generate_synthetic_data() -> DepthTimeline {
    // Price around $100.00 with $0.01 ticks
    // Spread of ~5-10 ticks
    // 30-50 bid/ask levels per snapshot, 30 snapshots

    let num_snapshots = 30;
    let base_price = 100.00_f32;

    let mut snapshots = Vec::with_capacity(num_snapshots);

    for t in 0..num_snapshots {
        let tick = (t * 100) as u64; // simulation ticks

        // Slowly drift the midpoint
        let drift = (t as f32 * 0.02).sin() * 0.10;
        let midpoint = base_price + drift;

        // Spread varies between 5-10 cents
        let spread_cents = 5 + (t % 6); // 5..10
        let half_spread = spread_cents as f32 * 0.01 / 2.0;

        let best_bid = midpoint - half_spread;
        let best_ask = midpoint + half_spread;

        // Generate 40 bid levels
        let num_bids = 40;
        let mut bids = Vec::with_capacity(num_bids);
        for i in 0..num_bids {
            let price = best_bid - i as f32 * 0.01;
            // Quantity: larger further from the spread, with some variation
            let base_qty = 50.0 + (i as f32 * 15.0);
            // Add time-varying noise
            let noise = ((t as f32 * 0.3 + i as f32 * 0.7).sin() * 0.5 + 0.5) * base_qty;
            let qty = base_qty + noise;
            // Some levels disappear occasionally
            if ((t + i) * 7) % 13 == 0 {
                continue;
            }
            bids.push(((price * 100.0).round() / 100.0, qty));
        }

        // Generate 40 ask levels
        let num_asks = 40;
        let mut asks = Vec::with_capacity(num_asks);
        for i in 0..num_asks {
            let price = best_ask + i as f32 * 0.01;
            let base_qty = 40.0 + (i as f32 * 12.0);
            let noise = ((t as f32 * 0.5 + i as f32 * 0.9).cos() * 0.5 + 0.5) * base_qty;
            let qty = base_qty + noise;
            if ((t + i) * 11) % 17 == 0 {
                continue;
            }
            asks.push(((price * 100.0).round() / 100.0, qty));
        }

        snapshots.push(DepthTimelineEntry { tick, bids, asks });
    }

    DepthTimeline { snapshots }
}

fn main() -> anyhow::Result<()> {
    let timeline = generate_synthetic_data();

    let palette = ColorPalette::from_theme(Theme::Light);

    let width = 1200;
    let height = 800;
    let column_width_px = 350.0; // wide columns so each histogram is clearly visible
    let visible_count = 3; // only show 3 snapshots

    let state = DepthTimelineState::new(timeline, visible_count, column_width_px);

    println!("Visible snapshots: {}", state.visible_snapshots().len());
    println!("Price range: ${:.2} - ${:.2}", state.price_min, state.price_max);

    let renderer = DepthTimelineRenderer::new(width, height, palette)?;

    let out_path = "/workspace/workspace/bigsoup/screenshots/depth_timeline.png";
    std::fs::create_dir_all("/workspace/workspace/bigsoup/screenshots").ok();

    renderer.render_to_png(&state, out_path)?;
    println!("Done! Open {} to see the depth timeline.", out_path);

    Ok(())
}
