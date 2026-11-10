//! cargo run -p gs-engine --example list_params -- <format> <name substring>
fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let d = gs_engine::plugins::scan_format_in_process(&a[1])?
        .into_iter()
        .find(|p| p.name.to_lowercase().contains(&a[2].to_lowercase()))
        .unwrap();
    let s = gs_engine::plugins::load_plugin(&d, 48_000.0, 1024)?;
    for p in &s.params {
        println!(
            "{:>4} id={:<10} {:<34} {:>8.2}..{:<8.2} steps={} {}",
            p.index, p.id, p.name, p.min, p.max, p.steps, p.unit
        );
    }
    Ok(())
}
