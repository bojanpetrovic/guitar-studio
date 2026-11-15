//! Guitar Studio as a plugin (VST3 and CLAP), so the whole rig — its plugins, graph,
//! pedalboard, scenes and presets — runs inside a DAW such as REAPER.
//!
//! Each plugin instance runs the same app layer as the desktop app (`gs-app`), with the
//! DAW driving the audio ([`gs_engine::audio::HostedAudio`]) instead of our own device
//! streams. The rig is saved with the DAW's project; presets and the plugin list are
//! shared with the desktop app.
//!
//! Built with [truce](https://github.com/truce-audio/truce); `make plugin` bundles it.

mod bundle;
mod editor;
mod host;

use gs_engine::audio::HostedAudio;
use host::Instance;
use std::sync::Arc;
use truce::core::buffer::AudioBuffer;
use truce::core::bus::BusLayout;
use truce::core::config::AudioConfig;
use truce::core::editor::EditorBuilder;
use truce::core::events::{EventList, TransportInfo};
use truce::core::export::PluginExport;
use truce::core::info::{AutomationConfig, MidiDialect, PluginCategory, PluginInfo};
use truce::core::meters::MeterStore;
use truce::core::plugin::PluginRuntime;
use truce::core::process::{ProcessContext, ProcessStatus};
use truce::core::snapshot::SnapshotSlot;
use truce::core::state::StateLoadError;

/// What the DAW can automate. The rig's own controls live in the plugin window.
#[derive(truce::Params)]
pub struct GuitarStudioParams {
    /// The rig's output level.
    #[param(name = "Output", range = "linear(-60, 12)", unit = "dB", default = 0.0)]
    pub output: truce::params::FloatParam,
}

/// Channels we process: stereo in, stereo out (a mono track is duplicated).
const CHANNELS: usize = 2;
/// Reported tail: delays and reverbs inside the rig keep ringing after the input stops.
const TAIL_SECONDS: f64 = 10.0;

pub struct GuitarStudio {
    instance: Instance,
    params: Arc<GuitarStudioParams>,
    meters: Arc<MeterStore>,
    /// The engine's audio side, while the DAW has us active.
    audio: Option<HostedAudio>,
    sample_rate: f64,
    /// Output gain applied at the end of the last block (ramped from, to the new one).
    gain: f32,
}

impl PluginRuntime for GuitarStudio {
    type Sample = f32;

    fn info() -> PluginInfo {
        PluginInfo {
            name: "Guitar Studio",
            vendor: "Guitar Studio",
            url: "",
            version: env!("CARGO_PKG_VERSION"),
            category: PluginCategory::Effect,
            accepts_midi_in: false,
            emits_midi: false,
            midi_input_dialect: MidiDialect::Midi1,
            midi_output_dialect: MidiDialect::Midi1,
            midi_input_ports: 0,
            midi_output_ports: 0,
            bundle_id: "dev.guitarstudio.plugin",
            vst3_id: "GuitarStudioRig1",
            clap_id: "dev.guitarstudio.plugin",
            fourcc: *b"GsRg",
            au_type: *b"aufx",
            au_manufacturer: *b"GtSt",
            aax_id: None,
            aax_category: None,
            vst3_subcategory: Some("Distortion"),
            preset_user_dir: None,
            vst3_name: None,
            clap_name: None,
            vst2_name: None,
            au_name: None,
            au3_name: None,
            aax_name: None,
            lv2_name: None,
            mute_preview_output: false,
            automation: AutomationConfig::DEFAULT,
            legacy_au_keys: &[],
            legacy_lv2_uris: &[],
            legacy_aax_chunk_ids: &[],
        }
    }

    fn bus_layouts() -> Vec<BusLayout> {
        BusLayout::stereo_and_mono()
    }

    /// The DAW (re)activated us, possibly at a new rate: run the engine on its audio.
    /// Not on the audio thread.
    fn reset(&mut self, config: &AudioConfig) {
        let block = config.max_block_size.max(1);
        self.sample_rate = config.sample_rate;
        let app = &self.instance.app;
        let rate = config.sample_rate.round() as u32;
        let started = app
            .studio
            .lock()
            .start_hosted(rate, CHANNELS as u16, block as u32);
        match started {
            Ok(audio) => self.audio = Some(audio),
            Err(e) => {
                log::error!("the engine didn't start: {e:#}");
                self.audio = None;
            }
        }
    }

    fn process(
        &mut self,
        buffer: &mut AudioBuffer<f32>,
        _events: &EventList,
        context: &mut ProcessContext,
    ) -> ProcessStatus {
        let frames = buffer.num_samples();
        let (inputs, ins, mut outputs, outs) = channels(buffer, frames);
        let Some(audio) = &mut self.audio else {
            outputs.iter_mut().for_each(|out| out.fill(0.0));
            return ProcessStatus::Normal;
        };
        // The engine reads a mono track into both sides and takes blocks of any length.
        let transport = host_transport(context.transport);
        audio.process(
            &inputs[..ins],
            &mut outputs[..outs],
            frames,
            transport.as_ref(),
        );
        self.apply_output_gain(&mut outputs[..outs], frames);
        // Delays and amp sims keep sounding without input: don't let the DAW put us to
        // sleep on silence.
        ProcessStatus::KeepAlive
    }

    fn load_state(&mut self, data: &[u8]) -> Result<(), StateLoadError> {
        self.instance.link.restore(data);
        Ok(())
    }

    fn tail(&self) -> u32 {
        (self.sample_rate * TAIL_SECONDS) as u32
    }
}

impl GuitarStudio {
    /// The Output parameter, ramped across the block so automation doesn't click.
    fn apply_output_gain(&mut self, outputs: &mut [&mut [f32]], frames: usize) {
        let db = self.params.output.raw_target();
        let target = if db <= -59.9 {
            0.0
        } else {
            10f32.powf(db as f32 / 20.0)
        };
        let start = self.gain;
        if start == 1.0 && target == 1.0 {
            return;
        }
        let step = (target - start) / frames.max(1) as f32;
        for out in outputs.iter_mut() {
            for (i, s) in out[..frames].iter_mut().enumerate() {
                *s *= start + step * (i + 1) as f32;
            }
        }
        self.gain = target;
    }
}

/// The DAW's first two input and output channels (`frames` long) side by side, with
/// how many of each there are.
fn channels<'a>(
    buffer: &'a mut AudioBuffer<'_, f32>,
    frames: usize,
) -> (
    [&'a [f32]; CHANNELS],
    usize,
    [&'a mut [f32]; CHANNELS],
    usize,
) {
    let ins = buffer.num_input_channels().min(CHANNELS);
    let outs = buffer.num_output_channels().min(CHANNELS);
    let mut input: [(*const f32, usize); CHANNELS] = [(std::ptr::null(), 0); CHANNELS];
    let mut output: [(*mut f32, usize); CHANNELS] = [(std::ptr::null_mut(), 0); CHANNELS];
    for (c, slot) in input.iter_mut().enumerate().take(ins) {
        let ch = buffer.input(c);
        *slot = (ch.as_ptr(), ch.len().min(frames));
    }
    for (c, slot) in output.iter_mut().enumerate().take(outs) {
        let ch = buffer.output(c);
        *slot = (ch.as_mut_ptr(), ch.len().min(frames));
    }
    // SAFETY: these are distinct channels of the DAW's buffer, valid for this block, and
    // `buffer` isn't used again while the slices live. No input aliases an output: we
    // don't opt into in-place processing, so the wrapper copies such inputs aside.
    let inputs = input.map(|(ptr, len)| {
        if ptr.is_null() {
            &[][..]
        } else {
            unsafe { std::slice::from_raw_parts(ptr, len) }
        }
    });
    let outputs = output.map(|(ptr, len)| {
        if ptr.is_null() {
            &mut [][..]
        } else {
            unsafe { std::slice::from_raw_parts_mut(ptr, len) }
        }
    });
    (inputs, ins, outputs, outs)
}

/// The DAW's transport, as hosted plugins expect it (`None` if it reports no tempo, in
/// which case the metronome's is used).
fn host_transport(t: &TransportInfo) -> Option<truce_rack::core::TransportInfo> {
    (t.tempo > 0.0).then(|| truce_rack::core::TransportInfo {
        tempo_bpm: Some(t.tempo),
        time_signature: (t.time_sig_num > 0 && t.time_sig_den > 0)
            .then(|| (u32::from(t.time_sig_num), u32::from(t.time_sig_den))),
        song_position_beats: Some(t.position_beats),
        song_position_samples: Some(t.position_samples),
        bar_start_beats: Some(t.bar_start_beats),
        playing: t.playing,
        recording: t.recording,
        loop_active: t.loop_active,
    })
}

impl PluginExport for GuitarStudio {
    type Params = GuitarStudioParams;

    fn create() -> Self {
        let log = if cfg!(windows) {
            Some(gs_app::local_dir().join("logs").join("plugin.log"))
        } else {
            std::env::var_os("HOME").map(|home| {
                std::path::PathBuf::from(home).join("Library/Logs/Guitar Studio/plugin.log")
            })
        };
        gs_app::logging::init(log.as_deref());
        // The DAW creates plugins on its main (UI) thread.
        gs_app::main_thread::init();
        Self {
            instance: Instance::new(),
            params: Arc::new(GuitarStudioParams::new()),
            meters: MeterStore::new(),
            audio: None,
            sample_rate: 48_000.0,
            gain: 1.0,
        }
    }

    fn params(&self) -> &GuitarStudioParams {
        &self.params
    }

    fn params_arc(&self) -> Arc<GuitarStudioParams> {
        self.params.clone()
    }

    fn meter_store(&self) -> Arc<MeterStore> {
        self.meters.clone()
    }

    fn snapshot_slot(&self) -> Arc<SnapshotSlot> {
        self.instance.link.snapshot.clone()
    }

    fn editor_builder(&self) -> EditorBuilder<GuitarStudioParams> {
        let instance = self.instance.clone();
        Box::new(move |_params| Some(Box::new(editor::WebEditor::new(&instance))))
    }

    fn has_editor_static() -> bool {
        true
    }
}

impl Drop for GuitarStudio {
    /// The DAW removed us: stop the engine and release every hosted plugin on the main
    /// thread (where plugins must be torn down).
    fn drop(&mut self) {
        self.audio = None;
        let app = self.instance.app.clone();
        if gs_app::main_thread::is_main() {
            app.shut_down();
        } else {
            gs_app::main_thread::run(move || app.shut_down());
        }
    }
}

#[cfg(feature = "clap")]
truce_clap::export_clap!(GuitarStudio);

#[cfg(feature = "vst3")]
truce_vst3::export_vst3!(GuitarStudio);
