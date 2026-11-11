//! cargo run -p gs-engine --example scan_catalog -- <scanner (gs-scan or the app)> <out.json>
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let t = std::time::Instant::now();
    let (plugins, errors) = gs_engine::plugins::scan_all_isolated(std::path::Path::new(&args[1]));
    eprintln!(
        "{} plugins, {} errors in {:?}",
        plugins.len(),
        errors.len(),
        t.elapsed()
    );
    for e in &errors {
        eprintln!("  {e}");
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let json = serde_json::json!({ "plugins": plugins, "errors": errors, "scannedAt": now, "scanning": false });
    std::fs::write(&args[2], serde_json::to_vec(&json).unwrap()).unwrap();
}
