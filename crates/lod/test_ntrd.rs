use lod::ntrd_decoder::NtrdReader;

fn main() {
    let mut reader = NtrdReader::open("/workspace/workspace/bb/beta_breaker/data/consolidated/hedgy-test/CIEN/2018-05-01_to_2025-09-18-trades.ntrd").unwrap();
    
    // Read first 10 trades
    let records = reader.read_records(0, 10).unwrap();
    
    for (i, r) in records.iter().enumerate() {
        println!("Trade {}: side_byte={:02x} ('{}'/'{}'), flags={:02x}, price={:.4}, size={}", 
                 i, r.side, r.side_char(), r.side_str(), r.flags, r.price_as_float(), r.size);
    }
    
    // Count different side values
    println!("\nChecking first 1000 trades for side values...");
    let records = reader.read_records(0, 1000).unwrap();
    let mut side_counts = std::collections::HashMap::new();
    for r in records.iter() {
        *side_counts.entry(r.side).or_insert(0) += 1;
    }
    
    println!("\nSide value counts:");
    for (side, count) in side_counts.iter() {
        println!("  0x{:02x} ('{}') : {} trades", side, *side as char, count);
    }
}
