//! Fast preset switching. Starting a plugin can take seconds (Archetype Nolly: ~2.3 s),
//! restoring a saved state into one that's already running takes milliseconds. So a
//! preset switch reuses running instances of the same plugins (see
//! `gs_engine::presets::instantiate_plugins`), and we keep spare instances warm:
//!
//! - instances the new rig doesn't use stay loaded if the previous or next preset in the
//!   list needs them (so stepping back and forth is instant);
//! - plugins the previous / next preset needs that aren't running yet are preloaded in
//!   the background, one at a time, once the switch has settled. AU plugins load right
//!   on the preload thread, so the UI stays responsive; VST3 / CLAP must use the main
//!   thread.
//!
//! Spares are capped at what those two neighbours need, so memory stays bounded.

use crate::{App, RigGuard, main_thread};
use gs_engine::nodes::MAX_BLOCK;
use gs_engine::plugins::{self, PluginDescriptor, SharedPlugin};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

/// Let a switch settle (and the user start playing) before preloading.
const SETTLE: Duration = Duration::from_millis(1500);
/// Breathing room between two preloads: each blocks the main thread while it starts.
const GAP: Duration = Duration::from_millis(300);

pub struct Warm {
    spares: Mutex<Vec<SharedPlugin>>,
    /// Bumped by every refresh; a preload run stops when it's no longer the latest.
    generation: AtomicU64,
    /// Held while a plugin preloads off the main thread, with its key in `in_flight`.
    loading: Mutex<()>,
    in_flight: Mutex<Option<String>>,
    /// Preloading is on. Inside a plugin host it waits until the user switches presets
    /// in the plugin window: a DAW project restores one rig per track, and each would
    /// otherwise quietly start its neighbours' plugins too.
    armed: AtomicBool,
}

impl Warm {
    /// `armed`: preload from the start (the desktop app) or only after [`Warm::arm`].
    pub fn new(armed: bool) -> Self {
        Self {
            spares: Mutex::default(),
            generation: AtomicU64::default(),
            loading: Mutex::default(),
            in_flight: Mutex::default(),
            armed: AtomicBool::new(armed),
        }
    }

    /// Start preloading neighbours (the user is switching presets).
    pub fn arm(&self) {
        self.armed.store(true, Ordering::Relaxed);
    }

    /// All spares, for a rig switch to pick from.
    pub fn take(&self) -> Vec<SharedPlugin> {
        std::mem::take(&mut *self.spares.lock())
    }

    /// Instances a rig switch didn't use. They're trimmed on the next refresh.
    pub fn put_back(&self, mut spares: Vec<SharedPlugin>) {
        self.spares.lock().append(&mut spares);
    }

    /// About to switch to a rig with these plugins: if one of them is preloading right
    /// now, stop preloading after it and wait for it — it's sooner than starting another.
    pub fn wait_for(&self, plugins: &[PluginDescriptor]) {
        let busy = self.in_flight.lock().clone();
        if busy.is_some_and(|key| plugins.iter().any(|d| d.key() == key)) {
            self.generation.fetch_add(1, Ordering::SeqCst);
            drop(self.loading.lock());
        }
    }

    /// Release every spare (e.g. the sample rate changed: they'd run at the old one).
    pub fn clear(&self, app: &App) {
        app.bury(self.take());
    }

    /// Stop any preloading (the app is shutting down).
    pub fn stop(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// Can `plugins` be set up without starting any plugin (all running or spare)?
    pub fn covers(&self, app: &App, plugins: &[PluginDescriptor]) -> bool {
        let mut have = counts(app.studio.lock().plugin_instances().iter());
        for (k, n) in counts(self.spares.lock().iter()) {
            *have.entry(k).or_default() += n;
        }
        let mut need: HashMap<String, usize> = HashMap::new();
        for d in plugins {
            *need.entry(d.key()).or_default() += 1;
        }
        need.iter()
            .all(|(k, n)| have.get(k).copied().unwrap_or(0) >= *n)
    }
}

fn counts<'a>(plugins: impl Iterator<Item = &'a SharedPlugin>) -> HashMap<String, usize> {
    let mut out: HashMap<String, usize> = HashMap::new();
    for p in plugins {
        *out.entry(p.lock().descriptor.key()).or_default() += 1;
    }
    out
}

/// The presets ← / → would go to from here (same wrap-around as the UI's stepper).
fn neighbours(names: &[String], current: Option<&str>) -> Vec<String> {
    let n = names.len() as isize;
    if n == 0 {
        return Vec::new();
    }
    let at = current
        .and_then(|c| names.iter().position(|x| x == c))
        .map_or(-1, |i| i as isize);
    let mut out: Vec<String> = [-1, 1]
        .iter()
        .map(|d| names[((at + d + n) % n) as usize].clone())
        .filter(|name| Some(name.as_str()) != current)
        .collect();
    out.dedup();
    out
}

/// After a switch (or a change to the preset list): drop spares nobody nearby needs and
/// preload what the neighbours will need, in the background.
pub fn refresh(app: &Arc<App>) {
    let generation = app.warm.generation.fetch_add(1, Ordering::SeqCst) + 1;
    if !app.warm.armed.load(Ordering::Relaxed) {
        return;
    }
    let app = Arc::downgrade(app);
    let spawned = std::thread::Builder::new()
        .name("gs-warm".into())
        .spawn(move || {
            std::thread::sleep(SETTLE);
            // The plugin host may have unloaded us meanwhile.
            let Some(state) = app.upgrade() else { return };
            let latest = || state.warm.generation.load(Ordering::SeqCst) == generation;
            if !latest() {
                return;
            }

            // How many spare instances of each plugin the neighbours need beyond the rig's.
            let names: Vec<String> = state.presets.list().into_iter().map(|p| p.name).collect();
            let current = state.current_preset.lock().clone();
            let running = counts(state.studio.lock().plugin_instances().iter());
            let mut wanted: HashMap<String, (usize, PluginDescriptor)> = HashMap::new();
            for name in neighbours(&names, current.as_deref()) {
                let Ok(plugins) = state.presets.plugins_of(&name) else {
                    continue;
                };
                let mut need: HashMap<String, (usize, PluginDescriptor)> = HashMap::new();
                for d in plugins {
                    need.entry(d.key()).or_insert((0, d)).0 += 1;
                }
                for (key, (n, d)) in need {
                    let extra = n.saturating_sub(running.get(&key).copied().unwrap_or(0));
                    let slot = wanted.entry(key).or_insert((0, d));
                    slot.0 = slot.0.max(extra);
                }
            }

            // Trim: keep up to `wanted` of each, release the rest (on the main thread).
            let mut missing = Vec::new();
            {
                let mut spares = state.warm.spares.lock();
                // A switch since we started wants other neighbours: leave it to that run.
                if !latest() {
                    return;
                }
                let mut kept: HashMap<String, usize> = HashMap::new();
                let mut released = Vec::new();
                spares.retain(|p| {
                    let key = p.lock().descriptor.key();
                    let k = kept.entry(key.clone()).or_default();
                    let keep = *k < wanted.get(&key).map_or(0, |w| w.0);
                    if keep {
                        *k += 1;
                    } else {
                        released.push(p.clone());
                    }
                    keep
                });
                state.bury(released);
                for (key, (n, d)) in &wanted {
                    let have = kept.get(key).copied().unwrap_or(0);
                    missing.extend(std::iter::repeat_n(d.clone(), n.saturating_sub(have)));
                }
            }

            // Preload, one plugin at a time. Whatever loads goes to the spares (the next
            // refresh trims what isn't needed after all).
            for desc in missing {
                if !latest() {
                    break;
                }
                state.emit("warming", &Some(&desc.name));
                if plugins::loads_off_main_thread(&desc) {
                    // Right here, off the main thread: the UI stays responsive however
                    // long it takes (Archetype Cory Wong: ~12 s).
                    let rate = state.studio.lock().sample_rate();
                    let _loading = state.warm.loading.lock();
                    *state.warm.in_flight.lock() = Some(desc.key());
                    let loaded = plugins::load_plugin(&desc, rate, MAX_BLOCK);
                    *state.warm.in_flight.lock() = None;
                    match loaded {
                        // Shut down meanwhile? Then nobody will take it: release it on
                        // the main thread, like every plugin.
                        Ok(slot) if !latest() => main_thread::run(move || drop(slot)),
                        Ok(slot) => state.warm.spares.lock().push(Arc::new(Mutex::new(slot))),
                        Err(e) => log::warn!("preloading {} failed: {e:#}", desc.name),
                    }
                    std::thread::sleep(GAP);
                    continue;
                }
                // VST3 / CLAP must start on the main thread (the window pauses meanwhile).
                std::thread::sleep(Duration::from_millis(80)); // let the UI show the chip first
                let main = state.clone();
                let ran = main_thread::run_and_wait(move || {
                    // Not in the middle of a rig change (a plugin pumping the run loop
                    // while it starts can run us nested); the next refresh catches up.
                    let Ok(_busy) = RigGuard::take() else { return };
                    let rate = main.studio.lock().sample_rate();
                    match plugins::load_plugin(&desc, rate, MAX_BLOCK) {
                        Ok(slot) => main.warm.spares.lock().push(Arc::new(Mutex::new(slot))),
                        Err(e) => log::warn!("preloading {} failed: {e:#}", desc.name),
                    }
                });
                if ran.is_none() {
                    break;
                }
                std::thread::sleep(GAP);
            }
            state.emit("warming", &None::<String>);
        });
    if let Err(e) = spawned {
        log::warn!("couldn't start the preload thread: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::neighbours;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn neighbours_wrap_like_the_ui() {
        let list = names(&["A", "B", "C", "D"]);
        assert_eq!(neighbours(&list, Some("B")), names(&["A", "C"]));
        assert_eq!(neighbours(&list, Some("A")), names(&["D", "B"]));
        assert_eq!(neighbours(&list, Some("D")), names(&["C", "A"]));
        // Untitled rig: → goes to the first, ← to the second-to-last (UI's formula).
        assert_eq!(neighbours(&list, None), names(&["C", "A"]));
        assert_eq!(neighbours(&names(&["A", "B"]), Some("A")), names(&["B"]));
        assert!(neighbours(&names(&["A"]), Some("A")).is_empty());
        assert!(neighbours(&[], None).is_empty());
    }
}
