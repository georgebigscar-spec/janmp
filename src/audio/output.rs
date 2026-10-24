//! Audio output: a cpal stream fed from a lock-free ring buffer.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};
use rtrb::{Consumer, Producer, RingBuffer};

use super::engine::Shared;

pub struct Output {
    _stream: cpal::Stream,
    pub producer: Producer<f32>,
    pub rate: u32,
    pub channels: usize,
}

impl Output {
    pub fn open(shared: Arc<Shared>) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or("no output device")?;
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let format = supported.sample_format();
        let buffer = *supported.buffer_size();
        let mut config: StreamConfig = supported.into();
        // Ask for ~40 ms device buffers: the default on some backends (ALSA → PulseAudio)
        // is several seconds, which makes play/seek feel laggy.
        let wanted = (config.sample_rate as u32 / 25).max(256);
        if let cpal::SupportedBufferSize::Range { min, max } = buffer {
            config.buffer_size = cpal::BufferSize::Fixed(wanted.clamp(min, max));
        }
        let rate = config.sample_rate as u32;
        let channels = config.channels as usize;
        // ~0.5 s of buffered audio: small enough for snappy seeking, large enough for jitter.
        let (producer, consumer) = RingBuffer::<f32>::new((rate as usize / 2) * channels);
        shared.out_rate.store(rate, Ordering::Relaxed);
        shared.out_channels.store(channels as u32, Ordering::Relaxed);

        let stream = match format {
            SampleFormat::F32 => build::<f32>(&device, &config, consumer, shared),
            SampleFormat::F64 => build::<f64>(&device, &config, consumer, shared),
            SampleFormat::I16 => build::<i16>(&device, &config, consumer, shared),
            SampleFormat::I32 => build::<i32>(&device, &config, consumer, shared),
            SampleFormat::U16 => build::<u16>(&device, &config, consumer, shared),
            SampleFormat::U8 => build::<u8>(&device, &config, consumer, shared),
            SampleFormat::I8 => build::<i8>(&device, &config, consumer, shared),
            other => Err(format!("unsupported output sample format {other}")),
        }?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Self { _stream: stream, producer, rate, channels })
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    mut consumer: Consumer<f32>,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let rate = config.sample_rate as f32;
    // Gain ramp used for click-free pause/resume (~15 ms).
    let ramp_step = 1.0 / (rate * 0.015);
    let mut gain = 0.0f32;
    let err_shared = shared.clone();

    device
        .build_output_stream(
            *config,
            move |data: &mut [T], _| {
                let req = shared.flush_req.load(Ordering::Acquire);
                if req != shared.flush_ack.load(Ordering::Relaxed) {
                    let n = consumer.slots();
                    if let Ok(chunk) = consumer.read_chunk(n) {
                        chunk.commit_all();
                    }
                    shared.played_frames.store(0, Ordering::Relaxed);
                    shared.flush_ack.store(req, Ordering::Release);
                }

                let paused = shared.paused.load(Ordering::Relaxed);
                let volume = f32::from_bits(shared.volume.load(Ordering::Relaxed));
                let balance = f32::from_bits(shared.balance.load(Ordering::Relaxed));
                let target = if paused { 0.0 } else { 1.0 };

                if paused && gain <= 0.0 {
                    data.fill(T::from_sample(0.0f32));
                    return;
                }

                let frames_wanted = data.len() / channels;
                let frames_avail = consumer.slots() / channels;
                let frames = frames_wanted.min(frames_avail);
                let mut written = 0usize;
                if frames > 0 {
                    if let Ok(chunk) = consumer.read_chunk(frames * channels) {
                        let (a, b) = chunk.as_slices();
                        let mut it = a.iter().chain(b.iter());
                        for frame in data.chunks_mut(channels).take(frames) {
                            if gain < target {
                                gain = (gain + ramp_step).min(1.0);
                            } else if gain > target {
                                gain = (gain - ramp_step).max(0.0);
                            }
                            let mut mono = 0.0;
                            for (ch, out) in frame.iter_mut().enumerate() {
                                let s = *it.next().unwrap_or(&0.0);
                                mono += s;
                                let pan = if channels == 2 {
                                    if ch == 0 { (1.0 - balance).min(1.0) } else { (1.0 + balance).min(1.0) }
                                } else {
                                    1.0
                                };
                                *out = T::from_sample(s * volume * gain * pan);
                            }
                            shared.vis.push(mono / channels as f32);
                        }
                        chunk.commit_all();
                        written = frames;
                    }
                }
                for s in &mut data[written * channels..] {
                    *s = T::from_sample(0.0f32);
                }
                shared.played_frames.fetch_add(written as u64, Ordering::Relaxed);
            },
            move |err| {
                log::warn!("audio stream error: {err}");
                if matches!(
                    err.kind(),
                    cpal::ErrorKind::DeviceNotAvailable | cpal::ErrorKind::DeviceChanged
                ) {
                    err_shared.stream_error.store(true, Ordering::Relaxed);
                }
            },
            None,
        )
        .map_err(|e| e.to_string())
}
