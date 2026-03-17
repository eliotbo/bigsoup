//! Generate a screenshot of the depth chart renderer with synthetic LOB data.
//! Usage: cargo run --bin depth_screenshot

use vizza::{DepthRenderer, DepthSnapshot};
use vizza::config::{ColorPalette, Theme};

fn main() -> anyhow::Result<()> {
    let palette = ColorPalette::from_theme(Theme::Light);
    let renderer = DepthRenderer::new(500, 800, palette)?;

    // Mid price $100.00, spread = 9 ticks at $0.01 = $0.09
    // Best bid = $99.96, best ask = $100.05
    let tick = 0.01_f32;
    let best_bid = 99.96_f32;
    let best_ask = 100.05_f32;

    let mut bids = Vec::new();
    let mut asks = Vec::new();

    // 50 bid levels below best bid, each $0.01 apart
    for i in 0..50 {
        let price = best_bid - i as f32 * tick;
        // Round to avoid float drift
        let price = (price * 100.0).round() / 100.0;
        // Quantity increases with distance from spread, with some noise
        let base_qty = 5.0 + (i as f32).powf(1.2) * 3.0;
        let qty = base_qty + ((i * 7) % 11) as f32 * 2.0;
        bids.push((price, qty));
    }

    // 50 ask levels above best ask, each $0.01 apart
    for i in 0..50 {
        let price = best_ask + i as f32 * tick;
        let price = (price * 100.0).round() / 100.0;
        let base_qty = 4.0 + (i as f32).powf(1.2) * 2.5;
        let qty = base_qty + ((i * 11) % 13) as f32 * 1.8;
        asks.push((price, qty));
    }

    let snapshot = DepthSnapshot { bids, asks };

    let out_path = "/workspace/workspace/bigsoup/depth_screenshot.png";
    renderer.render_to_png(&snapshot, out_path)?;
    println!("Done! Open {} to see the depth chart.", out_path);
    println!("Mid: $100.00, Spread: {} ticks (${})",
        ((best_ask - best_bid) / tick).round() as i32,
        best_ask - best_bid);

    Ok(())
}
