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
    /// The DAW's input, copied out so outputs can be written while reading it.
    inputs: [Vec<f32>; CHANNELS],
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
        for input in &mut self.inputs {
            input.resize(block, 0.0);
        }
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
        let outs = buffer.num_output_channels().min(CHANNELS);
        let Some(audio) = &mut self.audio else {
            for c in 0..outs {
                buffer.output(c).fill(0.0);
            }
            return ProcessStatus::Normal;
        };
        // More than the DAW promised at activation: play silence rather than allocate.
        if frames > self.inputs[0].len() {
            for c in 0..outs {
                buffer.output(c).fill(0.0);
            }
            return ProcessStatus::Normal;
        }
        let ins = buffer.num_input_channels().min(CHANNELS);
        for c in 0..CHANNELS {
            let dst = &mut self.inputs[c][..frames];
            if ins == 0 {
                dst.fill(0.0);
            } else {
                // A mono track feeds both sides.
                dst.copy_from_slice(&buffer.input(c.min(ins - 1))[..frames]);
            }
        }
        let inputs: [&[f32]; CHANNELS] = [&self.inputs[0][..frames], &self.inputs[1][..frames]];

        let mut raw: [(*mut f32, usize); CHANNELS] = [(std::ptr::null_mut(), 0); CHANNELS];
        for (c, slot) in raw.iter_mut().enumerate().take(outs) {
            let out = buffer.output(c);
            *slot = (out.as_mut_ptr(), out.len().min(frames));
        }
        // SAFETY: each pointer is a different output channel of the DAW's buffer (they
        // never overlap), valid for this block, and `buffer` isn't touched meanwhile.
        let mut outputs: [&mut [f32]; CHANNELS] = raw.map(|(ptr, len)| {
            if ptr.is_null() {
                &mut [][..]
            } else {
                unsafe { std::slice::from_raw_parts_mut(ptr, len) }
            }
        });
        let transport = host_transport(context.transport);
        audio.process(
            &inputs[..],
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
        let log = std::env::var_os("HOME").map(|home| {
            std::path::PathBuf::from(home).join("Library/Logs/Guitar Studio/plugin.log")
        });
        gs_app::logging::init(log.as_deref());
        Self {
            instance: Instance::new(),
            params: Arc::new(GuitarStudioParams::new()),
            meters: MeterStore::new(),
            audio: None,
            sample_rate: 48_000.0,
            inputs: [Vec::new(), Vec::new()],
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
