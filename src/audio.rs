use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, SupportedStreamConfig};
use hound::{WavSpec, WavWriter};
use std::fs::{create_dir_all, File};
use std::io::BufWriter;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub const MAX_BUFFER_SECS: f32 = 8.0;

pub struct CaptureState {
    pub wav: Option<WavWriter<BufWriter<File>>>,
    pub pcm: Vec<f32>,
    pub total_pushed: u64,
    pub channels: u16,
    pub sample_rate: u32,
}

pub type Capture = Arc<Mutex<CaptureState>>;

pub fn build_capture_stream(
    device: &Device,
    is_loopback: bool,
    wav_path: Option<&Path>,
) -> Result<(Stream, Capture), String> {
    let supported: SupportedStreamConfig = if is_loopback {
        device
            .default_output_config()
            .map_err(|e| format!("default_output_config: {e}"))?
    } else {
        device
            .default_input_config()
            .map_err(|e| format!("default_input_config: {e}"))?
    };

    let channels = supported.channels();
    let sample_rate: u32 = supported.sample_rate().into();
    let sample_format = supported.sample_format();

    let wav = if let Some(p) = wav_path {
        if let Some(parent) = p.parent() {
            let _ = create_dir_all(parent);
        }
        let spec = WavSpec {
            channels,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        Some(WavWriter::create(p, spec).map_err(|e| format!("create wav: {e}"))?)
    } else {
        None
    };

    let state = Arc::new(Mutex::new(CaptureState {
        wav,
        pcm: Vec::with_capacity((sample_rate * 6) as usize),
        total_pushed: 0,
        channels,
        sample_rate,
    }));

    let config = supported.config();
    let err_fn = |e| eprintln!("stream error: {e}");

    let stream = match sample_format {
        SampleFormat::F32 => {
            let s = state.clone();
            device.build_input_stream(
                &config,
                move |data: &[f32], _: &_| process(&s, data, |x| x),
                err_fn,
                None,
            )
        }
        SampleFormat::I16 => {
            let s = state.clone();
            device.build_input_stream(
                &config,
                move |data: &[i16], _: &_| process(&s, data, |x| x as f32 / 32768.0),
                err_fn,
                None,
            )
        }
        SampleFormat::U16 => {
            let s = state.clone();
            device.build_input_stream(
                &config,
                move |data: &[u16], _: &_| process(&s, data, |x| (x as f32 - 32768.0) / 32768.0),
                err_fn,
                None,
            )
        }
        SampleFormat::I32 => {
            let s = state.clone();
            device.build_input_stream(
                &config,
                move |data: &[i32], _: &_| process(&s, data, |x| x as f32 / 2_147_483_648.0),
                err_fn,
                None,
            )
        }
        other => return Err(format!("unsupported sample format: {other:?}")),
    }
    .map_err(|e| format!("build_input_stream: {e}"))?;

    stream.play().map_err(|e| format!("play: {e}"))?;
    Ok((stream, state))
}

fn process<T: Copy>(cap: &Capture, data: &[T], to_f32: impl Fn(T) -> f32) {
    let mut g = match cap.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let sample_rate = g.sample_rate;
    {
        let CaptureState {
            wav,
            pcm,
            total_pushed,
            channels,
            ..
        } = &mut *g;
        let chs = *channels as usize;
        if chs == 0 || data.is_empty() {
            return;
        }
        for frame in data.chunks_exact(chs) {
            let mut sum = 0.0f32;
            for &s in frame {
                let v = to_f32(s);
                sum += v;
                if let Some(w) = wav.as_mut() {
                    let _ = w.write_sample((v.clamp(-1.0, 1.0) * 32767.0) as i16);
                }
            }
            let mono = sum / chs as f32;
            pcm.push(mono);
            *total_pushed += 1;
        }
    }
    let max_keep = (sample_rate as f32 * MAX_BUFFER_SECS) as usize;
    if g.pcm.len() > max_keep {
        let excess = g.pcm.len() - max_keep;
        g.pcm.drain(..excess);
    }
}

/// Copy the most recent `n_samples` from the PCM buffer without consuming it.
/// Returns (samples_copy, total_pushed, sample_rate). The returned slice represents
/// audio from absolute sample position `total_pushed - samples_copy.len()` to `total_pushed`.
pub fn peek_tail(cap: &Capture, n_samples: usize) -> (Vec<f32>, u64, u32) {
    if let Ok(g) = cap.lock() {
        if g.pcm.is_empty() {
            return (Vec::new(), g.total_pushed, g.sample_rate);
        }
        let take = n_samples.min(g.pcm.len());
        let start = g.pcm.len() - take;
        (g.pcm[start..].to_vec(), g.total_pushed, g.sample_rate)
    } else {
        (Vec::new(), 0, 0)
    }
}

pub fn finalize_wav(cap: &Capture) {
    if let Ok(mut g) = cap.lock() {
        if let Some(w) = g.wav.take() {
            let _ = w.finalize();
        }
    }
}

pub fn pcm_len(cap: &Capture) -> usize {
    cap.lock().map(|g| g.pcm.len()).unwrap_or(0)
}

pub fn drain(cap: &Capture) -> (Vec<f32>, u64, u32) {
    if let Ok(mut g) = cap.lock() {
        let pcm = std::mem::take(&mut g.pcm);
        (pcm, g.total_pushed, g.sample_rate)
    } else {
        (Vec::new(), 0, 0)
    }
}

/// Drain the first `n` samples (up to buffer length). Returns drained samples,
/// the absolute sample position of the END of the drained chunk, and the sample rate.
pub fn drain_n(cap: &Capture, n: usize) -> (Vec<f32>, u64, u32) {
    if let Ok(mut g) = cap.lock() {
        let take = n.min(g.pcm.len());
        if take == 0 {
            return (Vec::new(), 0, g.sample_rate);
        }
        let drained: Vec<f32> = g.pcm.drain(..take).collect();
        let end_pos = g.total_pushed - g.pcm.len() as u64;
        (drained, end_pos, g.sample_rate)
    } else {
        (Vec::new(), 0, 0)
    }
}

/// Search the unconsumed PCM buffer for a silence boundary that would make a
/// good split point. Returns Some(split_index) within the buffer if found.
/// Scans backwards from `scan_end` to `min_split`, looking for `min_silence_ms`
/// of consecutive samples with RMS below `threshold`.
pub fn find_silence_split(
    cap: &Capture,
    min_split: usize,
    scan_end: usize,
    threshold: f32,
    min_silence_ms: u32,
) -> Option<usize> {
    let g = cap.lock().ok()?;
    let sr = g.sample_rate;
    if sr == 0 || g.pcm.is_empty() {
        return None;
    }
    let end = scan_end.min(g.pcm.len());
    if end <= min_split {
        return None;
    }
    let win = (sr as f32 * 0.05) as usize; // 50 ms window
    if win == 0 {
        return None;
    }
    let min_silent_wins = ((min_silence_ms as f32 / 50.0).ceil() as usize).max(1);

    let mut i = end;
    let mut silent_run = 0usize;
    while i >= min_split + win {
        i -= win;
        let window = &g.pcm[i..i + win];
        let sum_sq: f32 = window.iter().map(|s| s * s).sum();
        let rms = (sum_sq / win as f32).sqrt();
        if rms < threshold {
            silent_run += 1;
            if silent_run >= min_silent_wins {
                return Some(i + win);
            }
        } else {
            silent_run = 0;
        }
    }
    None
}
