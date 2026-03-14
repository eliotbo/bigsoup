//! Simple test to verify backfill functionality without opening a window.

use lod::LevelStore;
use std::sync::{Arc, Mutex};
use vizza::{MockBackfillSource, live_view::LiveDataManager};

fn main() -> anyhow::Result<()> {
    println!("=== Backfill Test ===");
    println!();

    // Create a mock source with backfill support
    let source = Box::new(MockBackfillSource::new());

    // Create a level store (empty for this test)
    let level_store = Arc::new(Mutex::new(LevelStore::new()));

    // Create LiveDataManager with today-so-far enabled
    let mut manager = LiveDataManager::with_data_source_and_options(
        level_store,
        source,
        "TEST",
        true, // Enable today-so-far
    );

    println!("Initializing backfill...");
    println!();

    // Initialize backfill
    match manager.initialize_with_backfill() {
        Ok(_) => {
            println!();
            println!("✓ Backfill test completed successfully!");
            println!(
                "  - Backfill completed: {}",
                manager.is_backfill_completed()
            );
            println!(
                "  - Today-so-far enabled: {}",
                manager.is_today_so_far_enabled()
            );
        }
        Err(e) => {
            eprintln!("✗ Backfill test failed: {}", e);
            return Err(anyhow::anyhow!("Backfill failed: {}", e));
        }
    }

    Ok(())
}
