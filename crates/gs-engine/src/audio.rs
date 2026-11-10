//! Device I/O (cpal / CoreAudio) and the real-time callback.
//!
//! cpal opens input and output as separate streams, so the input callback pushes
//! samples into a lock-free ring that the output callback drains. The output side
//! caps how much audio may queue up, so the round trip stays near one buffer.

use crate::graph::Schedule;
use crate::metronome::{Metronome, MetronomeShared};
use crate::nodes::{MAX_BLOCK, ProcessCtx};
use crate::util::{AtomicF32, peak};
use anyhow::{Context, Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Instant;
use truce_rack::core::TransportInfo;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AudioConfig {
    /// Device name; `None` = system default.
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    /// Frames per callback.
    pub buffer_size: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub name: String,
    pub channels: u16,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceList {
    pub inputs: Vec<DeviceInfo>,
    pub outputs: Vec<DeviceInfo>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioStatus {
    pub running: bool,
    pub input_device: String,
    pub output_device: String,
    pub input_channels: u16,
    pub output_channels: u16,
    pub sample_rate: u32,
    pub buffer_size: u32,
    /// Estimated round trip (input buffer + output buffer + queue), in ms.
    pub latency_ms: f32,
}

pub fn list_devices() -> DeviceList {
    let host = cpal::default_host();
    let default_in = host.default_input_device().map(|d| d.to_string());
    let default_out = host.default_output_device().map(|d| d.to_string());
    let collect = |devices: Vec<cpal::Device>, input: bool, default: &Option<String>| {
        devices
            .into_iter()
            .filter_map(|d| {
                let cfg = if input {
                    d.default_input_config()
                } else {
                    d.default_output_config()
                }
                .ok()?;
                let name = d.to_string();
                Some(DeviceInfo {
                    is_default: Some(&name) == default.as_ref(),
                    name,
                    channels: cfg.channels(),
                })
            })
            .collect::<Vec<_>>()
    };
    DeviceList {
        inputs: collect(
            host.input_devices()
                .map(|i| i.collect())
                .unwrap_or_default(),
            true,
            &default_in,
        ),
        outputs: collect(
            host.output_devices()
                .map(|i| i.collect())
                .unwrap_or_default(),
            false,
            &default_out,
        ),
    }
}

fn find_device(input: bool, name: &Option<String>) -> Result<cpal::Device> {
    let host = cpal::default_host();
    if let Some(name) = name {
        let mut devices: Vec<cpal::Device> = if input {
            host.input_devices()?.collect()
        } else {
            host.output_devices()?.collect()
        };
        if let Some(pos) = devices.iter().position(|d| &d.to_string() == name) {
            return Ok(devices.swap_remove(pos));
        }
    }
    let dev = if input {
        host.default_input_device()
    } else {
        host.default_output_device()
    };
    dev.ok_or_else(|| {
        anyhow!(
            "no {} device available",
            if input { "input" } else { "output" }
        )
    })
}

fn supported_rates(dev: &cpal::Device, input: bool) -> Vec<(u32, u32)> {
    let ranges: Vec<(u32, u32)> = if input {
        dev.supported_input_configs()
            .map(|it| {
                it.map(|r| (r.min_sample_rate(), r.max_sample_rate()))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        dev.supported_output_configs()
            .map(|it| {
                it.map(|r| (r.min_sample_rate(), r.max_sample_rate()))
                    .collect()
            })
            .unwrap_or_default()
    };
    ranges
}

/// Both streams must run at the same rate: prefer the output's current rate, then the
/// input's, then common studio rates.
fn negotiate_rate(
    in_dev: &cpal::Device,
    out_dev: &cpal::Device,
    in_default: &cpal::SupportedStreamConfig,
    out_default: &cpal::SupportedStreamConfig,
) -> Result<u32> {
    let ins = supported_rates(in_dev, true);
    let outs = supported_rates(out_dev, false);
    let ok = |ranges: &[(u32, u32)], r: u32| ranges.iter().any(|(lo, hi)| (*lo..=*hi).contains(&r));
    // Prefer a rate that leaves both devices as they are; never switch a device to a
    // non-music rate (webcams offer 16–32 kHz, and agreeing on those would reconfigure
    // the user's interface system-wide).
    let candidates = [
        out_default.sample_rate(),
        in_default.sample_rate(),
        48_000,
        44_100,
        96_000,
        88_200,
    ];
    candidates.into_iter().filter(|r| *r >= 44_100).find(|r| ok(&ins, *r) && ok(&outs, *r)).ok_or_else(|| {
        let list = |v: &[(u32, u32)]| v.iter().map(|(lo, _)| format!("{}k", *lo as f32 / 1000.0)).collect::<Vec<_>>().join("/");
        anyhow!(
            "'{in_dev}' ({}) and '{out_dev}' ({}) share no usable sample rate (44.1 kHz or higher) — choose a different input or output in Audio settings",
            list(&ins),
            list(&outs)
        )
    })
}

/// Messages from the control thread to the audio thread.
pub enum Command {
    SetSchedule(Box<Schedule>),
}

/// Meters and load figures written by the audio thread.
#[derive(Debug, Default)]
pub struct Telemetry {
    pub output_peak: [AtomicF32; 2],
    /// Fraction of the callback period spent processing (EMA).
    pub cpu_load: AtomicF32,
    pub underruns: AtomicU32,
}

pub struct AudioShared {
    pub telemetry: Arc<Telemetry>,
    pub metronome: Arc<MetronomeShared>,
    pub tuner_enabled: Arc<AtomicBool>,
    /// Which hardware input channel feeds the tuner (follows the Input node).
    pub tuner_channel: Arc<AtomicU32>,
}

/// A running pair of streams plus the control-side ends of their queues.
pub struct RunningAudio {
    _input: cpal::Stream,
    _output: cpal::Stream,
    commands: rtrb::Producer<Command>,
    garbage: rtrb::Consumer<Box<Schedule>>,
    pub status: AudioStatus,
}

impl RunningAudio {
    pub fn send_schedule(&mut self, schedule: Schedule) -> Result<()> {
        self.commands
            .push(Command::SetSchedule(Box::new(schedule)))
            .map_err(|_| anyhow!("audio command queue full"))
    }

    /// Free schedules the audio thread has retired. Call regularly from a non-RT thread.
    pub fn collect_garbage(&mut self) {
        while let Ok(old) = self.garbage.pop() {
            drop(old);
        }
    }
}

/// Opens devices and starts streams. `tuner_tx` receives the tuner channel's samples.
pub fn start(
    config: &AudioConfig,
    shared: AudioShared,
    initial: Schedule,
    tuner_tx: rtrb::Producer<f32>,
) -> Result<RunningAudio> {
    let in_dev = find_device(true, &config.input_device)?;
    let out_dev = find_device(false, &config.output_device)?;
    let out_default = out_dev
        .default_output_config()
        .context("output device config")?;
    let in_default = in_dev
        .default_input_config()
        .context("input device config")?;
    let sample_rate = negotiate_rate(&in_dev, &out_dev, &in_default, &out_default)?;
    let buffer_size = config.buffer_size.clamp(16, MAX_BLOCK as u32);

    let in_channels = in_default.channels();
    let out_channels = out_default.channels();

    // Use a fixed buffer size when the device reports a range; clamp into it.
    let pick_buffer = |supported: &cpal::SupportedBufferSize| match supported {
        cpal::SupportedBufferSize::Range { min, max } => {
            cpal::BufferSize::Fixed(buffer_size.clamp(*min, (*max).min(MAX_BLOCK as u32)))
        }
        cpal::SupportedBufferSize::Unknown => cpal::BufferSize::Default,
    };
    let in_cfg = cpal::StreamConfig {
        channels: in_channels,
        sample_rate,
        buffer_size: pick_buffer(in_default.buffer_size()),
    };
    let out_cfg = cpal::StreamConfig {
        channels: out_channels,
        sample_rate,
        buffer_size: pick_buffer(out_default.buffer_size()),
    };
    let effective_buffer = match out_cfg.buffer_size {
        cpal::BufferSize::Fixed(n) => n,
        cpal::BufferSize::Default => buffer_size,
    };

    let ring_len = in_channels as usize * MAX_BLOCK * 8;
    let (mut in_tx, in_rx) = rtrb::RingBuffer::<f32>::new(ring_len);
    let (cmd_tx, cmd_rx) = rtrb::RingBuffer::<Command>::new(64);
    let (garbage_tx, garbage_rx) = rtrb::RingBuffer::<Box<Schedule>>::new(64);

    let input = in_dev
        .build_input_stream::<f32, _, _>(
            in_cfg,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                let n = data.len().min(in_tx.slots());
                if let Ok(chunk) = in_tx.write_chunk_uninit(n) {
                    chunk.fill_from_iter(data.iter().copied());
                }
            },
            |e| log::error!("input stream error: {e}"),
            None,
        )
        .context("opening input stream (is microphone access allowed?)")?;

    let mut rt = Rt {
        schedule: Box::new(initial),
        commands: cmd_rx,
        garbage: garbage_tx,
        input: in_rx,
        in_channels: in_channels as usize,
        out_channels: out_channels as usize,
        hw_in: vec![vec![0.0; MAX_BLOCK]; in_channels as usize],
        interleaved: vec![0.0; MAX_BLOCK * in_channels as usize],
        master: [vec![0.0; MAX_BLOCK], vec![0.0; MAX_BLOCK]],
        click: vec![0.0; MAX_BLOCK],
        metronome: Metronome::default(),
        shared,
        tuner_tx,
        sample_rate: sample_rate as f32,
        max_queue_frames: effective_buffer as usize * 2,
        song_samples: 0,
        song_beats: 0.0,
    };
    let output = out_dev
        .build_output_stream::<f32, _, _>(
            out_cfg,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| rt.process(data),
            |e| log::error!("output stream error: {e}"),
            None,
        )
        .context("opening output stream")?;

    input.play().context("starting input")?;
    output.play().context("starting output")?;

    let latency_ms = 3.0 * effective_buffer as f32 / sample_rate as f32 * 1000.0;
    Ok(RunningAudio {
        _input: input,
        _output: output,
        commands: cmd_tx,
        garbage: garbage_rx,
        status: AudioStatus {
            running: true,
            input_device: in_dev.to_string(),
            output_device: out_dev.to_string(),
            input_channels: in_channels,
            output_channels: out_channels,
            sample_rate,
            buffer_size: effective_buffer,
            latency_ms,
        },
    })
}

/// Audio-thread state, owned by the output callback.
struct Rt {
    schedule: Box<Schedule>,
    commands: rtrb::Consumer<Command>,
    garbage: rtrb::Producer<Box<Schedule>>,
    input: rtrb::Consumer<f32>,
    in_channels: usize,
    out_channels: usize,
    hw_in: Vec<Vec<f32>>,
    interleaved: Vec<f32>,
    master: [Vec<f32>; 2],
    click: Vec<f32>,
    metronome: Metronome,
    shared: AudioShared,
    tuner_tx: rtrb::Producer<f32>,
    sample_rate: f32,
    max_queue_frames: usize,
    song_samples: i64,
    song_beats: f64,
}

impl Rt {
    fn process(&mut self, data: &mut [f32]) {
        let started = Instant::now();
        while let Ok(cmd) = self.commands.pop() {
            match cmd {
                Command::SetSchedule(s) => {
                    let old = std::mem::replace(&mut self.schedule, s);
                    if let Err(rtrb::PushError::Full(old)) = self.garbage.push(old) {
                        // Never free on the audio thread; leak instead (queue is sized so this won't happen).
                        std::mem::forget(old);
                    }
                }
            }
        }

        let total_frames = data.len() / self.out_channels.max(1);
        let mut offset = 0;
        while offset < total_frames {
            let n = (total_frames - offset).min(MAX_BLOCK);
            self.read_input(n);
            self.tap_tuner(n);
            self.run_block(n);
            self.write_output(
                &mut data[offset * self.out_channels..(offset + n) * self.out_channels],
                n,
            );
            offset += n;
        }

        let period = total_frames as f32 / self.sample_rate;
        if period > 0.0 {
            let load = started.elapsed().as_secs_f32() / period;
            let t = &self.shared.telemetry.cpu_load;
            t.store(t.load() * 0.9 + load * 0.1);
        }
    }

    fn read_input(&mut self, n: usize) {
        let ch = self.in_channels;
        let want = n * ch;
        // Drop stale input so latency can't creep up if clocks drift.
        let max_queued = (self.max_queue_frames + n) * ch;
        let queued = self.input.slots();
        if queued > max_queued {
            let skip = (queued - n * ch) / ch * ch;
            if let Ok(chunk) = self.input.read_chunk(skip) {
                chunk.commit_all();
            }
        }
        let have = self.input.slots().min(want) / ch.max(1) * ch;
        let buf = &mut self.interleaved[..want];
        if let Ok(chunk) = self.input.read_chunk(have) {
            let (a, b) = chunk.as_slices();
            buf[..a.len()].copy_from_slice(a);
            buf[a.len()..a.len() + b.len()].copy_from_slice(b);
            chunk.commit_all();
        }
        if have < want {
            buf[have..].fill(0.0);
            if have == 0 || have < want {
                self.shared
                    .telemetry
                    .underruns
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
        for c in 0..ch {
            let dst = &mut self.hw_in[c][..n];
            for (f, d) in dst.iter_mut().enumerate() {
                *d = buf[f * ch + c];
            }
        }
    }

    fn tap_tuner(&mut self, n: usize) {
        if !self.shared.tuner_enabled.load(Ordering::Relaxed) {
            return;
        }
        let c = (self.shared.tuner_channel.load(Ordering::Relaxed) as usize)
            .min(self.in_channels.saturating_sub(1));
        let Some(src) = self.hw_in.get(c) else { return };
        let k = n.min(self.tuner_tx.slots());
        if let Ok(chunk) = self.tuner_tx.write_chunk_uninit(k) {
            chunk.fill_from_iter(src[..k].iter().copied());
        }
    }

    fn run_block(&mut self, n: usize) {
        let met = &self.shared.metronome;
        let bpm = met.bpm.load().max(1.0) as f64;
        let transport = TransportInfo {
            tempo_bpm: Some(bpm),
            time_signature: Some((met.beats_per_bar.load(Ordering::Relaxed), 4)),
            song_position_beats: Some(self.song_beats),
            song_position_samples: Some(self.song_samples),
            bar_start_beats: None,
            playing: true,
            recording: false,
            loop_active: false,
        };
        let ctx = ProcessCtx {
            sample_rate: self.sample_rate,
            frames: n,
            hw_input: &self.hw_in,
            transport,
        };
        let [ml, mr] = &mut self.master;
        self.schedule.run(&ctx, [&mut ml[..n], &mut mr[..n]]);

        let click = &mut self.click[..n];
        click.fill(0.0);
        self.metronome.process(met, self.sample_rate, click);
        for i in 0..n {
            ml[i] += click[i];
            mr[i] += click[i];
        }

        self.song_samples += n as i64;
        self.song_beats += n as f64 / (self.sample_rate as f64 * 60.0 / bpm);
        let tel = &self.shared.telemetry;
        tel.output_peak[0].raise_to(peak(&ml[..n]));
        tel.output_peak[1].raise_to(peak(&mr[..n]));
    }

    fn write_output(&mut self, out: &mut [f32], n: usize) {
        let ch = self.out_channels;
        let [l, r] = &self.master;
        for f in 0..n {
            let frame = &mut out[f * ch..(f + 1) * ch];
            // Soft safety clip so a runaway plugin can't blast the speakers.
            let (sl, sr) = (l[f].clamp(-1.0, 1.0), r[f].clamp(-1.0, 1.0));
            match ch {
                1 => frame[0] = 0.5 * (sl + sr),
                _ => {
                    frame[0] = sl;
                    frame[1] = sr;
                    frame[2..].fill(0.0);
                }
            }
        }
    }
}
