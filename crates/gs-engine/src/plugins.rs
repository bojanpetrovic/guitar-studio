//! Plugin discovery, loading and activation (VST3, AU v2, AU v3, CLAP via truce-rack).
//!
//! Scanning runs in a **child process**: some plugins crash when probed, and that must
//! not take the studio down. The app re-executes itself with `--scan-plugins <format>`
//! (see [`run_scan_child`]) and reads JSON from its stdout.

use crate::nodes::MAX_PLUGIN_CHANNELS;
use anyhow::{Context, Result, anyhow, bail};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use truce_rack::core::buffer::BusRange;
use truce_rack::core::{BusKind, BusLayout, Plugin, PluginCategory, PluginInfo, PluginScanner};

pub const FORMATS: &[&str] = &["vst3", "au", "au3", "clap"];
const SCAN_MARKER: &str = "@@GS-SCAN@@";

/// Serializable mirror of truce-rack's `PluginInfo`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginDescriptor {
    pub name: String,
    pub vendor: String,
    pub version: u32,
    pub category: String,
    pub path: PathBuf,
    pub unique_id: String,
    pub format: String,
    pub has_editor: bool,
    pub accepts_midi: bool,
}

impl PluginDescriptor {
    /// Stable key used by the UI and patches.
    pub fn key(&self) -> String {
        format!("{}:{}", self.format, self.unique_id)
    }

    fn from_info(info: &PluginInfo) -> Self {
        Self {
            name: info.name.clone(),
            vendor: info.vendor.clone(),
            version: info.version,
            category: category_name(info.category).to_string(),
            path: info.path.clone(),
            unique_id: info.unique_id.clone(),
            format: info.format.to_string(),
            has_editor: info.has_editor,
            accepts_midi: info.accepts_midi,
        }
    }

    fn to_info(&self) -> Result<PluginInfo> {
        Ok(PluginInfo {
            name: self.name.clone(),
            vendor: self.vendor.clone(),
            version: self.version,
            category: category_from_name(&self.category),
            path: self.path.clone(),
            unique_id: self.unique_id.clone(),
            format: static_format(&self.format)?,
            has_editor: self.has_editor,
            accepts_midi: self.accepts_midi,
        })
    }
}

fn category_name(c: PluginCategory) -> &'static str {
    match c {
        PluginCategory::Effect => "effect",
        PluginCategory::Instrument => "instrument",
        PluginCategory::NoteEffect => "noteEffect",
        PluginCategory::Analyzer => "analyzer",
        PluginCategory::Tool => "tool",
    }
}

fn category_from_name(s: &str) -> PluginCategory {
    match s {
        "instrument" => PluginCategory::Instrument,
        "noteEffect" => PluginCategory::NoteEffect,
        "analyzer" => PluginCategory::Analyzer,
        "tool" => PluginCategory::Tool,
        _ => PluginCategory::Effect,
    }
}

fn static_format(s: &str) -> Result<&'static str> {
    FORMATS
        .iter()
        .copied()
        .find(|f| *f == s)
        .ok_or_else(|| anyhow!("unknown plugin format '{s}'"))
}

/// Scan one format in-process. Only call this from the scan child process.
pub fn scan_format_in_process(format: &str) -> Result<Vec<PluginDescriptor>> {
    let infos = match format {
        "vst3" => truce_rack::vst3::Vst3Scanner::new().scan()?,
        "clap" => truce_rack::clap::ClapScanner::new().scan()?,
        #[cfg(target_vendor = "apple")]
        "au" => truce_rack::au::AuScanner::new().scan()?,
        #[cfg(target_vendor = "apple")]
        "au3" => truce_rack::au3::Au3Scanner::new().scan()?,
        other => bail!("format '{other}' is not supported on this platform"),
    };
    Ok(infos.iter().map(PluginDescriptor::from_info).collect())
}

/// Scan a single bundle or directory in-process (VST3 / CLAP only).
pub fn scan_path_in_process(format: &str, path: &Path) -> Result<Vec<PluginDescriptor>> {
    let infos = match format {
        "vst3" => truce_rack::vst3::Vst3Scanner::new().scan_path(path)?,
        "clap" => truce_rack::clap::ClapScanner::new().scan_path(path)?,
        other => bail!("path scans aren't supported for '{other}'"),
    };
    Ok(infos.iter().map(PluginDescriptor::from_info).collect())
}

/// Entry point for the `--scan-plugins <format>` child process: prints JSON to stdout.
pub fn run_scan_child(format: &str) -> i32 {
    match scan_format_in_process(format) {
        Ok(list) => {
            // Plugins sometimes write to stdout themselves; tag our line so the parent can find it.
            println!(
                "\n{SCAN_MARKER}{}",
                serde_json::to_string(&list).unwrap_or_else(|_| "[]".into())
            );
            0
        }
        Err(e) => {
            eprintln!("scan of {format} failed: {e:#}");
            1
        }
    }
}

/// Scan all formats, each in its own child process of `exe`. A crashing format
/// scan is reported in the returned error list rather than aborting the whole scan.
pub fn scan_all_isolated(exe: &Path) -> (Vec<PluginDescriptor>, Vec<String>) {
    let mut all = Vec::new();
    let mut errors = Vec::new();
    for format in FORMATS {
        if !cfg!(target_vendor = "apple") && format.starts_with("au") {
            continue;
        }
        match Command::new(exe).args(["--scan-plugins", format]).output() {
            Ok(out) if out.status.success() => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let json = stdout
                    .lines()
                    .find_map(|l| l.strip_prefix(SCAN_MARKER))
                    .unwrap_or("");
                match serde_json::from_str::<Vec<PluginDescriptor>>(json) {
                    Ok(list) => all.extend(list),
                    Err(e) => errors.push(format!("{format}: bad scan output ({e})")),
                }
            }
            Ok(out) => errors.push(format!(
                "{format}: scanner exited with {} — {}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
                    .lines()
                    .last()
                    .unwrap_or("")
            )),
            Err(e) => errors.push(format!("{format}: could not start scanner ({e})")),
        }
    }
    // VST3 bundles often omit the vendor; borrow it from another format's entry of the same plugin.
    let vendors: std::collections::HashMap<String, String> = all
        .iter()
        .filter(|p| !p.vendor.is_empty())
        .map(|p| (p.name.to_lowercase(), p.vendor.clone()))
        .collect();
    for p in all.iter_mut().filter(|p| p.vendor.is_empty()) {
        if let Some(v) = vendors.get(&p.name.to_lowercase()) {
            p.vendor = v.clone();
        }
    }
    // AU v3 scans also surface v2 components; keep one entry per unique id, preferring v2.
    let mut seen = std::collections::HashSet::new();
    all.retain(|p| {
        let id = if p.format.starts_with("au") {
            format!("au:{}", p.unique_id)
        } else {
            p.key()
        };
        seen.insert(id)
    });
    all.sort_by_key(|a| a.name.to_lowercase());
    (all, errors)
}

/// How the plugin's (fixed, post-activation) bus layout maps onto flat channels.
#[derive(Debug, Clone)]
pub struct PluginIo {
    pub total_in: usize,
    pub total_out: usize,
    /// Channel count of the main input bus (0 for instruments).
    pub main_in: usize,
    /// Channel count of the main output bus.
    pub main_out: usize,
    pub bus_in: Vec<BusRange>,
    pub bus_out: Vec<BusRange>,
}

/// A loaded, activated plugin.
pub struct PluginSlot {
    pub plugin: Box<dyn Plugin<f32>>,
    pub io: PluginIo,
    pub descriptor: PluginDescriptor,
    /// Parameter metadata, read once at load. Index = the plugin's parameter index.
    pub params: Vec<ParamInfo>,
}

/// A plugin parameter as the UI sees it. `id` is the format's stable parameter id
/// (what pedalboard bindings store); values are in the parameter's native range.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamInfo {
    pub index: usize,
    pub id: u32,
    pub name: String,
    pub unit: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    /// Number of discrete values (0 = continuous).
    pub steps: u32,
    pub is_bypass: bool,
}

impl PluginSlot {
    pub fn param_index(&self, id: u32) -> Option<usize> {
        self.params.iter().position(|p| p.id == id)
    }
}

fn read_params(plugin: &dyn Plugin<f32>) -> Vec<ParamInfo> {
    use truce_rack::core::info::ParameterFlags;
    (0..plugin.parameter_count())
        .filter_map(|index| {
            let p = plugin.parameter_info(index).ok()?;
            if p.flags.contains(ParameterFlags::HIDDEN)
                || p.flags.contains(ParameterFlags::READ_ONLY)
            {
                return None;
            }
            Some(ParamInfo {
                index,
                id: p.id,
                name: if p.name.is_empty() {
                    format!("Param {index}")
                } else {
                    p.name
                },
                unit: p.unit,
                min: p.min,
                max: p.max,
                default: p.default,
                steps: p.step_count,
                is_bypass: p.flags.contains(ParameterFlags::BYPASS),
            })
        })
        .collect()
}

pub type SharedPlugin = Arc<Mutex<PluginSlot>>;

/// Load and activate a plugin. On macOS call this on the **main thread** — AU v3
/// instantiation and VST3 controllers expect it.
pub fn load_plugin(
    desc: &PluginDescriptor,
    sample_rate: f64,
    max_block: usize,
) -> Result<PluginSlot> {
    let info = desc.to_info()?;
    let mut plugin: Box<dyn Plugin<f32>> = match desc.format.as_str() {
        "vst3" => Box::new(truce_rack::vst3::Vst3Scanner::new().load(&info)?),
        "clap" => Box::new(truce_rack::clap::ClapScanner::new().load(&info)?),
        #[cfg(target_vendor = "apple")]
        "au" => Box::new(truce_rack::au::AuScanner::new().load(&info)?),
        #[cfg(target_vendor = "apple")]
        "au3" => Box::new(truce_rack::au3::Au3Scanner::new().load(&info)?),
        other => bail!("format '{other}' is not supported"),
    };
    let layout = choose_layout(plugin.supported_layouts());
    let io = plugin_io(&layout)?;
    plugin
        .activate(layout, sample_rate, max_block)
        .with_context(|| format!("activating {}", desc.name))?;
    // Scanners don't instantiate plugins, so they always report `has_editor = false`.
    // The loaded instance knows the truth (VST3 probes createView, AU checks CocoaUI,
    // CLAP checks the gui extension).
    let live = plugin.info();
    let descriptor = PluginDescriptor {
        has_editor: live.has_editor,
        accepts_midi: live.accepts_midi || desc.accepts_midi,
        ..desc.clone()
    };
    let params = read_params(plugin.as_ref());
    Ok(PluginSlot {
        plugin,
        io,
        descriptor,
        params,
    })
}

/// Re-activate after a sample-rate change.
pub fn reactivate(slot: &mut PluginSlot, sample_rate: f64, max_block: usize) -> Result<()> {
    let layout = slot
        .plugin
        .active_layout()
        .cloned()
        .unwrap_or_else(|| choose_layout(slot.plugin.supported_layouts()));
    if slot.plugin.is_active() {
        slot.plugin.deactivate();
    }
    slot.plugin.activate(layout, sample_rate, max_block)?;
    Ok(())
}

/// Prefer stereo→stereo, then mono→stereo, then mono→mono, then whatever is first.
fn choose_layout(layouts: &[BusLayout]) -> BusLayout {
    let main = |l: &BusLayout| {
        let i = l
            .inputs
            .first()
            .filter(|b| b.kind == BusKind::Main)
            .map_or(0, |b| b.channels.count());
        let o = l.outputs.first().map_or(0, |b| b.channels.count());
        (i, o)
    };
    for want in [(2, 2), (1, 2), (1, 1)] {
        if let Some(l) = layouts.iter().find(|l| main(l) == want) {
            return l.clone();
        }
    }
    layouts.first().cloned().unwrap_or_else(BusLayout::stereo)
}

fn plugin_io(layout: &BusLayout) -> Result<PluginIo> {
    fn ranges(buses: &[truce_rack::core::bus::Bus]) -> (Vec<BusRange>, usize) {
        let mut start = 0;
        let r = buses
            .iter()
            .map(|b| {
                let len = b.channels.count() as usize;
                let range = BusRange::new(start, len);
                start += len;
                range
            })
            .collect();
        (r, start)
    }
    let (bus_in, total_in) = ranges(&layout.inputs);
    let (bus_out, total_out) = ranges(&layout.outputs);
    if total_in > MAX_PLUGIN_CHANNELS || total_out > MAX_PLUGIN_CHANNELS {
        bail!("plugin needs more than {MAX_PLUGIN_CHANNELS} channels");
    }
    let main_in = layout
        .inputs
        .first()
        .filter(|b| b.kind == BusKind::Main)
        .map_or(0, |b| b.channels.count() as usize);
    let main_out = layout
        .outputs
        .first()
        .map_or(0, |b| b.channels.count() as usize);
    Ok(PluginIo {
        total_in,
        total_out,
        main_in,
        main_out,
        bus_in,
        bus_out,
    })
}
