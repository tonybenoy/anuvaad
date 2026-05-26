use crate::audio::{peek_tail, Capture};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

#[derive(Clone)]
pub struct TranslationRequest {
    pub line_id: u64,
    pub text: String,
    pub src: String,
    pub tgt: String,
}

#[derive(Clone)]
pub struct TranslationConfig {
    pub enabled: bool,
    pub src: String,
    pub tgt: String,
    /// 2-letter Whisper language code, or None for auto-detect.
    pub whisper_lang: Option<String>,
}

impl Default for TranslationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            src: "eng_Latn".to_string(),
            tgt: "eng_Latn".to_string(),
            whisper_lang: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    Mic,
    Sys,
}

impl Source {
    pub fn tag(&self) -> &'static str {
        match self {
            Source::Mic => "MIC",
            Source::Sys => "SYS",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranscribeMode {
    Off,
    MicOnly,
    SystemOnly,
    Both,
}

impl TranscribeMode {
    pub fn label(&self) -> &'static str {
        match self {
            TranscribeMode::Off => "Off",
            TranscribeMode::MicOnly => "Mic only",
            TranscribeMode::SystemOnly => "System only",
            TranscribeMode::Both => "Both (mic + system)",
        }
    }
}

#[derive(Clone)]
pub struct TranscriptLine {
    pub id: u64,
    pub start_s: f64,
    pub end_s: f64,
    pub source: Source,
    pub text: String,
    pub translation: Option<String>,
}

#[derive(Default)]
pub struct LiveTranscript {
    pub committed: Vec<TranscriptLine>,
    pub provisional_mic: Vec<TranscriptLine>,
    pub provisional_sys: Vec<TranscriptLine>,
    pub next_id: u64,
}

impl LiveTranscript {
    pub fn clear(&mut self) {
        self.committed.clear();
        self.provisional_mic.clear();
        self.provisional_sys.clear();
        self.next_id = 0;
    }
}

pub fn load_context(path: &str) -> Result<Arc<WhisperContext>, String> {
    let ctx = WhisperContext::new_with_params(path, WhisperContextParameters::default())
        .map_err(|e| format!("load whisper model: {e}"))?;
    Ok(Arc::new(ctx))
}

pub struct Worker {
    pub stop: Arc<AtomicBool>,
    pub handle: JoinHandle<()>,
}

const STEP_MS: u64 = 1200;
const WINDOW_SECS: f32 = 5.0;
const PROVISIONAL_SECS: f64 = 2.0;
const VOICE_RMS: f32 = 0.005;

struct SourceProc {
    committed_until_s: f64,
}

pub fn spawn_worker(
    ctx: Arc<WhisperContext>,
    streams: Vec<(Source, Capture)>,
    transcript: Arc<Mutex<LiveTranscript>>,
    translator_tx: Option<Sender<TranslationRequest>>,
    translation_cfg: Arc<Mutex<TranslationConfig>>,
) -> Worker {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_c = stop.clone();
    let handle = std::thread::spawn(move || {
        let mut state = match ctx.create_state() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("create_state: {e}");
                return;
            }
        };
        let threads = (num_cpus::get() as i32).clamp(1, 8);
        let mut procs: HashMap<Source, SourceProc> = streams
            .iter()
            .map(|(s, _)| {
                (
                    *s,
                    SourceProc {
                        committed_until_s: 0.0,
                    },
                )
            })
            .collect();

        loop {
            let stopping = stop_c.load(Ordering::Relaxed);
            let pass_start = Instant::now();

            for (src, cap) in &streams {
                let proc = procs.get_mut(src).expect("source state");
                process_window(
                    &mut state,
                    threads,
                    cap,
                    *src,
                    &transcript,
                    proc,
                    stopping,
                    translator_tx.as_ref(),
                    &translation_cfg,
                );
            }

            if stopping {
                break;
            }

            let elapsed = pass_start.elapsed();
            if let Some(left) = Duration::from_millis(STEP_MS).checked_sub(elapsed) {
                std::thread::sleep(left);
            }
        }
    });
    Worker { stop, handle }
}

fn process_window(
    state: &mut WhisperState,
    threads: i32,
    cap: &Capture,
    source: Source,
    transcript: &Arc<Mutex<LiveTranscript>>,
    proc: &mut SourceProc,
    final_flush: bool,
    translator_tx: Option<&Sender<TranslationRequest>>,
    translation_cfg: &Arc<Mutex<TranslationConfig>>,
) {
    let sr = match cap.lock() {
        Ok(g) => g.sample_rate,
        Err(_) => return,
    };
    if sr == 0 {
        return;
    }

    let window_samples = (WINDOW_SECS * sr as f32) as usize;
    let (samples, end_pos, _) = peek_tail(cap, window_samples);
    let min_samples = (sr as f32 * 1.0) as usize;
    if samples.len() < min_samples && !final_flush {
        return;
    }
    if samples.is_empty() {
        return;
    }

    let window_end_s = end_pos as f64 / sr as f64;
    let window_start_s = window_end_s - samples.len() as f64 / sr as f64;
    let window_dur_s = samples.len() as f64 / sr as f64;

    let resampled = resample_linear(&samples, sr, 16_000);
    if resampled.len() < 16_000 / 2 {
        return;
    }

    let rms = (resampled.iter().map(|s| s * s).sum::<f32>() / resampled.len() as f32).sqrt();
    if rms < VOICE_RMS {
        clear_provisional(transcript, source);
        return;
    }

    let lang_str = translation_cfg
        .lock()
        .ok()
        .and_then(|g| g.whisper_lang.clone());
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_n_threads(threads);
    params.set_language(lang_str.as_deref());
    params.set_translate(false);
    params.set_print_progress(false);
    params.set_print_special(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_no_context(true);
    params.set_suppress_blank(true);
    params.set_single_segment(false);

    if let Err(e) = state.full(params, &resampled) {
        eprintln!("whisper full: {e}");
        return;
    }

    let n = state.full_n_segments();
    let cutoff = if final_flush {
        f64::INFINITY
    } else {
        window_end_s - PROVISIONAL_SECS
    };

    let mut new_committed: Vec<TranscriptLine> = Vec::new();
    let mut new_provisional: Vec<TranscriptLine> = Vec::new();

    for i in 0..n {
        let Some(seg) = state.get_segment(i) else {
            continue;
        };
        let text = seg.to_str_lossy().unwrap_or_default();
        let trimmed = text.trim();
        if trimmed.is_empty() || is_noise(trimmed) {
            continue;
        }

        let t0 = (seg.start_timestamp() as f64 * 0.01).clamp(0.0, window_dur_s);
        let t1 = (seg.end_timestamp() as f64 * 0.01).clamp(t0, window_dur_s);
        let abs_start = window_start_s + t0;
        let abs_end = window_start_s + t1;

        if abs_end <= proc.committed_until_s {
            continue;
        }

        let line = TranscriptLine {
            id: 0, // assigned at commit time
            start_s: abs_start,
            end_s: abs_end,
            source,
            text: trimmed.to_string(),
            translation: None,
        };

        if abs_end <= cutoff {
            new_committed.push(line);
        } else {
            new_provisional.push(line);
        }
    }

    let mut translation_jobs: Vec<TranslationRequest> = Vec::new();
    if let Ok(mut t) = transcript.lock() {
        match source {
            Source::Mic => t.provisional_mic = new_provisional,
            Source::Sys => t.provisional_sys = new_provisional,
        }
        let cfg = translation_cfg.lock().ok().map(|g| g.clone());
        for mut line in new_committed {
            proc.committed_until_s = proc.committed_until_s.max(line.end_s);
            line.id = t.next_id;
            t.next_id += 1;
            if let (Some(tx), Some(cfg)) = (translator_tx, cfg.as_ref()) {
                if cfg.enabled && !line.text.trim().is_empty() {
                    let _ = tx; // silence unused if not enabled
                    translation_jobs.push(TranslationRequest {
                        line_id: line.id,
                        text: line.text.clone(),
                        src: cfg.src.clone(),
                        tgt: cfg.tgt.clone(),
                    });
                }
            }
            t.committed.push(line);
        }
    }

    if let Some(tx) = translator_tx {
        for job in translation_jobs {
            let _ = tx.send(job);
        }
    }
}

fn clear_provisional(transcript: &Arc<Mutex<LiveTranscript>>, source: Source) {
    if let Ok(mut t) = transcript.lock() {
        match source {
            Source::Mic => t.provisional_mic.clear(),
            Source::Sys => t.provisional_sys.clear(),
        }
    }
}

fn is_noise(s: &str) -> bool {
    let l = s.trim().to_lowercase();
    if l.is_empty() {
        return true;
    }
    if l.starts_with('[') && l.ends_with(']') {
        return true;
    }
    if l.starts_with('(') && l.ends_with(')') {
        return true;
    }
    matches!(
        l.as_str(),
        "you" | "thank you." | "thanks for watching." | "thanks for watching!"
    )
}

fn resample_linear(input: &[f32], in_rate: u32, out_rate: u32) -> Vec<f32> {
    if in_rate == out_rate {
        return input.to_vec();
    }
    let ratio = out_rate as f64 / in_rate as f64;
    let out_len = (input.len() as f64 * ratio) as usize;
    if out_len == 0 || input.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(out_len);
    let last = input.len() - 1;
    for i in 0..out_len {
        let src = i as f64 / ratio;
        let lo = (src as usize).min(last);
        let hi = (lo + 1).min(last);
        let frac = (src - lo as f64) as f32;
        out.push(input[lo] * (1.0 - frac) + input[hi] * frac);
    }
    out
}
