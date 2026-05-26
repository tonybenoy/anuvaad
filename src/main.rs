#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod model;
mod sessions;
mod transcribe;
mod translator;

use audio::{build_capture_stream, finalize_wav, Capture};
use cpal::traits::{DeviceTrait, HostTrait};
use cpal::{Device, Stream};
use eframe::egui;
use model::DownloadHandle;
use sessions::SessionMeta;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use transcribe::{
    spawn_worker, LiveTranscript, Source, TranscribeMode, TranscriptLine, TranslationConfig, Worker,
};
use translator::{Translator, LANGUAGES};

#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Record,
    Sessions,
    Compact,
}

const ACCENT: egui::Color32 = egui::Color32::from_rgb(180, 140, 255);
const SURFACE: egui::Color32 = egui::Color32::from_rgb(30, 30, 38);
const SURFACE_ALT: egui::Color32 = egui::Color32::from_rgb(40, 40, 50);
const TEXT_DIM: egui::Color32 = egui::Color32::from_gray(150);
const TEXT: egui::Color32 = egui::Color32::from_gray(225);
const MIC_COLOR: egui::Color32 = egui::Color32::from_rgb(120, 200, 255);
const SYS_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 200, 120);
const SUCCESS: egui::Color32 = egui::Color32::from_rgb(120, 210, 150);
const TRANS_COLOR: egui::Color32 = egui::Color32::from_rgb(160, 220, 180);

struct AudioRecorder {
    mic_devices: Vec<(String, Device)>,
    out_devices: Vec<(String, Device)>,
    mic_idx: usize,
    out_idx: usize,
    output_dir: String,
    model_path: String,

    recording: bool,
    record_start: Option<Instant>,
    mic_stream: Option<Stream>,
    sys_stream: Option<Stream>,
    mic_cap: Option<Capture>,
    sys_cap: Option<Capture>,
    last_files: Option<(PathBuf, PathBuf, PathBuf)>,

    whisper: Option<Arc<whisper_rs::WhisperContext>>,
    whisper_path_loaded: Option<String>,
    transcript: Arc<Mutex<LiveTranscript>>,
    worker: Option<Worker>,
    transcribe_mode: TranscribeMode,
    translator: Arc<Mutex<Option<Arc<Translator>>>>,
    translator_loading: Arc<Mutex<bool>>,
    translator_status: Arc<Mutex<String>>,
    translation_cfg: Arc<Mutex<TranslationConfig>>,

    download: Option<DownloadHandle>,

    status: String,
    autoloaded: bool,

    view: ViewMode,
    session_cache: Vec<SessionMeta>,
    session_cache_dir: String,
    selected_session: Option<usize>,
    selected_transcript: Option<String>,
    compact_always_on_top: bool,
    pending_view_switch: Option<ViewMode>,
    rename_buffer: String,
    rename_target_id: Option<String>,
}

impl Default for AudioRecorder {
    fn default() -> Self {
        let host = cpal::default_host();
        let mic_devices: Vec<(String, Device)> = host
            .input_devices()
            .map(|it| it.collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|d| d.name().ok().map(|n| (n, d)))
            .collect();
        let out_devices: Vec<(String, Device)> = host
            .output_devices()
            .map(|it| it.collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|d| d.name().ok().map(|n| (n, d)))
            .collect();
        let default_mic = host.default_input_device().and_then(|d| d.name().ok());
        let default_out = host.default_output_device().and_then(|d| d.name().ok());
        let mic_idx = default_mic
            .as_ref()
            .and_then(|n| mic_devices.iter().position(|(name, _)| name == n))
            .unwrap_or(0);
        let out_idx = default_out
            .as_ref()
            .and_then(|n| out_devices.iter().position(|(name, _)| name == n))
            .unwrap_or(0);

        let model_path = model::default_model_path().to_string_lossy().into_owned();
        let model_exists = std::path::Path::new(&model_path).exists();

        Self {
            mic_devices,
            out_devices,
            mic_idx,
            out_idx,
            output_dir: dirs_music_or_cwd(),
            model_path,
            recording: false,
            record_start: None,
            mic_stream: None,
            sys_stream: None,
            mic_cap: None,
            sys_cap: None,
            last_files: None,
            whisper: None,
            whisper_path_loaded: None,
            transcript: Arc::new(Mutex::new(LiveTranscript::default())),
            worker: None,
            transcribe_mode: TranscribeMode::SystemOnly,
            translator: Arc::new(Mutex::new(None)),
            translator_loading: Arc::new(Mutex::new(false)),
            translator_status: Arc::new(Mutex::new("translator not started".to_string())),
            translation_cfg: Arc::new(Mutex::new(TranslationConfig::default())),
            download: None,
            status: if model_exists {
                "Loading model…".to_string()
            } else {
                "No model found. Click \"Download base model\" to fetch one (~140 MB).".to_string()
            },
            autoloaded: false,
            view: ViewMode::Record,
            session_cache: Vec::new(),
            session_cache_dir: String::new(),
            selected_session: None,
            selected_transcript: None,
            compact_always_on_top: true,
            pending_view_switch: None,
            rename_buffer: String::new(),
            rename_target_id: None,
        }
    }
}

fn dirs_music_or_cwd() -> String {
    if let Some(home) = std::env::var_os("USERPROFILE") {
        let p = PathBuf::from(home).join("Music").join("anuvaad");
        return p.to_string_lossy().into_owned();
    }
    std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| ".".to_string())
}

impl AudioRecorder {
    fn start(&mut self) {
        if self.mic_devices.is_empty() || self.out_devices.is_empty() {
            self.status = "No audio devices found".to_string();
            return;
        }
        let ts = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
        let session_dir = PathBuf::from(&self.output_dir).join(&ts);
        if let Err(e) = std::fs::create_dir_all(&session_dir) {
            self.status = format!("Could not create session folder: {e}");
            return;
        }
        let mic_path = session_dir.join("mic.wav");
        let sys_path = session_dir.join("system.wav");
        let txt_path = session_dir.join("transcript.txt");

        let mic_dev = &self.mic_devices[self.mic_idx].1;
        let sys_dev = &self.out_devices[self.out_idx].1;

        let (mic_stream, mic_cap) = match build_capture_stream(mic_dev, false, Some(&mic_path)) {
            Ok(v) => v,
            Err(e) => {
                self.status = format!("Mic failed: {e}");
                return;
            }
        };
        let (sys_stream, sys_cap) = match build_capture_stream(sys_dev, true, Some(&sys_path)) {
            Ok(v) => v,
            Err(e) => {
                drop(mic_stream);
                finalize_wav(&mic_cap);
                self.status = format!("System loopback failed: {e}");
                return;
            }
        };

        self.mic_stream = Some(mic_stream);
        self.sys_stream = Some(sys_stream);
        self.mic_cap = Some(mic_cap.clone());
        self.sys_cap = Some(sys_cap.clone());
        self.last_files = Some((mic_path, sys_path, txt_path));
        self.record_start = Some(Instant::now());
        if let Ok(mut t) = self.transcript.lock() {
            t.clear();
        }

        let streams = match self.transcribe_mode {
            TranscribeMode::Off => Vec::new(),
            TranscribeMode::MicOnly => vec![(Source::Mic, mic_cap)],
            TranscribeMode::SystemOnly => vec![(Source::Sys, sys_cap)],
            TranscribeMode::Both => vec![(Source::Mic, mic_cap), (Source::Sys, sys_cap)],
        };

        if !streams.is_empty() {
            if let Some(ctx) = &self.whisper {
                let trans_tx = self
                    .translator
                    .lock()
                    .ok()
                    .and_then(|g| g.as_ref().map(|t| t.sender()));
                self.worker = Some(spawn_worker(
                    ctx.clone(),
                    streams,
                    self.transcript.clone(),
                    trans_tx,
                    self.translation_cfg.clone(),
                ));
                self.status = format!(
                    "Recording + transcribing ({})…",
                    self.transcribe_mode.label()
                );
            } else {
                self.status = "Recording (no transcription — model not loaded)…".to_string();
            }
        } else {
            self.status = "Recording (transcription off)…".to_string();
        }
        self.recording = true;
    }

    fn stop(&mut self) {
        if let Some(w) = self.worker.take() {
            w.stop.store(true, Ordering::Relaxed);
            let _ = w.handle.join();
        }
        let duration_s = self
            .record_start
            .map(|t| t.elapsed().as_secs_f64())
            .unwrap_or(0.0);
        self.mic_stream = None;
        self.sys_stream = None;
        if let Some(c) = &self.mic_cap {
            finalize_wav(c);
        }
        if let Some(c) = &self.sys_cap {
            finalize_wav(c);
        }

        if let Some((_, _, txt)) = &self.last_files {
            let _ = self.save_transcript(txt);
        }

        if let Some((mic_p, _, _)) = &self.last_files {
            if let Some(session_dir) = mic_p.parent() {
                let id = session_dir
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let started_iso = chrono::Local::now()
                    .checked_sub_signed(chrono::Duration::milliseconds(
                        (duration_s * 1000.0) as i64,
                    ))
                    .unwrap_or_else(chrono::Local::now)
                    .to_rfc3339();
                let mic_dev = self
                    .mic_devices
                    .get(self.mic_idx)
                    .map(|(n, _)| n.as_str())
                    .unwrap_or("?");
                let sys_dev = self
                    .out_devices
                    .get(self.out_idx)
                    .map(|(n, _)| n.as_str())
                    .unwrap_or("?");
                let cfg = self.translation_cfg.lock().ok().map(|g| g.clone());
                let (te, tsrc, ttgt, wlang) = cfg
                    .map(|c| (c.enabled, c.src, c.tgt, c.whisper_lang))
                    .unwrap_or((false, String::new(), String::new(), None));
                let _ = sessions::write_metadata(
                    session_dir,
                    &id,
                    None,
                    &started_iso,
                    duration_s,
                    mic_dev,
                    sys_dev,
                    self.transcribe_mode.label(),
                    wlang.as_deref(),
                    te,
                    &tsrc,
                    &ttgt,
                );
            }
        }

        let summary = self
            .last_files
            .as_ref()
            .and_then(|(m, _, _)| m.parent().map(|p| p.to_path_buf()))
            .map(|d| format!("Session saved: {}", d.display()))
            .unwrap_or_else(|| "Stopped".to_string());
        self.status = summary;

        self.recording = false;
        self.record_start = None;
        self.mic_cap = None;
        self.sys_cap = None;
        self.session_cache_dir.clear(); // force rescan on next Sessions view
    }

    fn save_transcript(&self, path: &PathBuf) -> std::io::Result<()> {
        use std::io::Write;
        let t = self.transcript.lock().unwrap();
        // After stop(), final_flush has committed everything, so committed is the full transcript.
        if t.committed.is_empty() && t.provisional_mic.is_empty() && t.provisional_sys.is_empty() {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let mut f = std::fs::File::create(path)?;
        let mut all: Vec<&TranscriptLine> = t
            .committed
            .iter()
            .chain(t.provisional_mic.iter())
            .chain(t.provisional_sys.iter())
            .collect();
        all.sort_by(|a, b| {
            a.start_s
                .partial_cmp(&b.start_s)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for line in all {
            writeln!(
                f,
                "[{:>7.2}s] [{}] {}",
                line.start_s,
                line.source.tag(),
                line.text
            )?;
            if let Some(tr) = &line.translation {
                writeln!(f, "          → {tr}")?;
            }
        }
        Ok(())
    }

    fn render_compact(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("anuvaad")
                    .size(13.0)
                    .strong()
                    .color(ACCENT),
            );
            if self.recording {
                if let Some(t) = self.record_start {
                    let e = t.elapsed().as_secs();
                    ui.label(
                        egui::RichText::new(format!("● {:02}:{:02}", e / 60, e % 60))
                            .size(13.0)
                            .color(egui::Color32::from_rgb(220, 80, 80)),
                    );
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("⛶").on_hover_text("Expand").clicked() {
                    self.pending_view_switch = Some(ViewMode::Record);
                }
                let aot_label = if self.compact_always_on_top {
                    "📌"
                } else {
                    "📍"
                };
                if ui
                    .small_button(aot_label)
                    .on_hover_text("Toggle always-on-top")
                    .clicked()
                {
                    self.compact_always_on_top = !self.compact_always_on_top;
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                            if self.compact_always_on_top {
                                egui::WindowLevel::AlwaysOnTop
                            } else {
                                egui::WindowLevel::Normal
                            },
                        ));
                }
                let rec_label = if self.recording { "⏹" } else { "⏺" };
                let rec_color = if self.recording {
                    egui::Color32::from_rgb(220, 80, 80)
                } else {
                    SUCCESS
                };
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new(rec_label).size(15.0).color(rec_color),
                        )
                        .min_size(egui::vec2(28.0, 22.0)),
                    )
                    .on_hover_text(if self.recording { "Stop" } else { "Record" })
                    .clicked()
                {
                    if self.recording {
                        self.stop();
                    } else {
                        self.start();
                    }
                }
            });
        });

        ui.add_space(2.0);
        ui.separator();

        egui::ScrollArea::vertical()
            .id_salt("compact_transcript")
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                let t = self.transcript.lock().unwrap();
                let recent = t.committed.iter().rev().take(8).rev();
                let mut any = false;
                for line in recent {
                    any = true;
                    ui.add_space(2.0);
                    ui.label(egui::RichText::new(&line.text).size(12.5).color(TEXT));
                    if let Some(tr) = &line.translation {
                        ui.label(
                            egui::RichText::new(format!("→ {tr}"))
                                .size(11.5)
                                .italics()
                                .color(TRANS_COLOR),
                        );
                    }
                }
                let prov: Vec<&TranscriptLine> = t
                    .provisional_mic
                    .iter()
                    .chain(t.provisional_sys.iter())
                    .collect();
                for line in prov {
                    any = true;
                    ui.add_space(2.0);
                    ui.label(
                        egui::RichText::new(&line.text)
                            .size(12.5)
                            .color(TEXT_DIM)
                            .italics(),
                    );
                }
                if !any {
                    ui.weak("(waiting for audio…)");
                }
            });
    }

    fn render_sessions(&mut self, ui: &mut egui::Ui) {
        if self.session_cache_dir != self.output_dir {
            self.session_cache = sessions::discover(std::path::Path::new(&self.output_dir));
            self.session_cache_dir = self.output_dir.clone();
            self.selected_session = None;
            self.selected_transcript = None;
        }

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("📂 {}", self.output_dir)).color(TEXT_DIM));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Open folder").clicked() {
                    let _ = std::process::Command::new("explorer")
                        .arg(&self.output_dir)
                        .spawn();
                }
                if ui.button("🔄 Refresh").clicked() {
                    self.session_cache = sessions::discover(std::path::Path::new(&self.output_dir));
                    self.selected_session = None;
                    self.selected_transcript = None;
                }
            });
        });
        ui.add_space(6.0);

        if self.session_cache.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.label(
                    egui::RichText::new("no sessions yet")
                        .size(16.0)
                        .color(TEXT_DIM),
                );
                ui.label(egui::RichText::new("Switch to 🎙 Record and create one.").color(TEXT_DIM));
            });
            return;
        }

        let total_h = ui.available_height();
        ui.horizontal_top(|ui| {
            // Left: session list
            ui.allocate_ui_with_layout(
                egui::vec2(280.0, total_h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    egui::Frame::group(ui.style()).fill(SURFACE).show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("sess_list")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                for i in 0..self.session_cache.len() {
                                    let s = &self.session_cache[i];
                                    let selected = Some(i) == self.selected_session;
                                    let resp = session_card(ui, s, selected);
                                    if resp.clicked() {
                                        self.selected_session = Some(i);
                                        self.selected_transcript = sessions::read_transcript(s);
                                    }
                                }
                            });
                    });
                },
            );

            ui.add_space(8.0);

            // Right: detail
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), total_h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    egui::Frame::group(ui.style()).fill(SURFACE).show(ui, |ui| {
                        if let Some(i) = self.selected_session {
                            let s = self.session_cache[i].clone();
                            let transcript = self.selected_transcript.clone();
                            let action = self.render_session_detail(ui, &s, transcript.as_deref());
                            if let Some(act) = action {
                                self.apply_session_action(act, &s);
                            }
                        } else {
                            ui.vertical_centered(|ui| {
                                ui.add_space(60.0);
                                ui.label(
                                    egui::RichText::new("Select a session to view").color(TEXT_DIM),
                                );
                            });
                        }
                    });
                },
            );
        });
    }

    fn start_translator(&mut self) {
        if self
            .translator
            .lock()
            .ok()
            .map(|g| g.is_some())
            .unwrap_or(false)
        {
            return;
        }
        if *self.translator_loading.lock().unwrap() {
            return;
        }
        *self.translator_loading.lock().unwrap() = true;
        *self.translator_status.lock().unwrap() =
            "Starting translator (first run may download ~600MB)…".to_string();

        let translator_slot = self.translator.clone();
        let loading_flag = self.translator_loading.clone();
        let status = self.translator_status.clone();
        let transcript = self.transcript.clone();

        std::thread::spawn(move || {
            let script = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                .map(|p| p.join("scripts").join("translator.py"))
                .filter(|p| p.exists())
                .or_else(|| {
                    let candidate = PathBuf::from("scripts/translator.py");
                    if candidate.exists() {
                        Some(candidate)
                    } else {
                        None
                    }
                });

            let Some(script) = script else {
                *status.lock().unwrap() = "translator.py not found".to_string();
                *loading_flag.lock().unwrap() = false;
                return;
            };

            match Translator::spawn("python", &script, transcript) {
                Ok(t) => {
                    let device = t.device.clone();
                    *translator_slot.lock().unwrap() = Some(Arc::new(t));
                    *status.lock().unwrap() = format!("translator ready ({device})");
                }
                Err(e) => {
                    *status.lock().unwrap() = format!("translator failed: {e}");
                }
            }
            *loading_flag.lock().unwrap() = false;
        });
    }

    fn load_model(&mut self) {
        let path = self.model_path.clone();
        if !std::path::Path::new(&path).exists() {
            self.status = "Model file not found at given path".to_string();
            return;
        }
        self.status = "Loading model…".to_string();
        match transcribe::load_context(&path) {
            Ok(ctx) => {
                self.whisper = Some(ctx);
                self.whisper_path_loaded = Some(path.clone());
                self.status = format!("Model loaded: {path}");
            }
            Err(e) => {
                self.status = format!("Model load failed: {e}");
            }
        }
    }

    fn start_download(&mut self) {
        self.start_download_url(model::MODEL_URL.to_string());
    }

    fn start_download_url(&mut self, url: String) {
        if self.download.is_some() {
            return;
        }
        let dest = PathBuf::from(&self.model_path);
        self.download = Some(model::download_model_async(dest, url));
        self.status = "Downloading model…".to_string();
    }

    fn model_loaded(&self) -> bool {
        self.whisper.is_some()
            && self.whisper_path_loaded.as_deref() == Some(self.model_path.as_str())
    }
}

impl Drop for AudioRecorder {
    fn drop(&mut self) {
        if self.recording {
            self.stop();
        }
    }
}

#[allow(deprecated)]
impl eframe::App for AudioRecorder {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        if !self.autoloaded {
            self.autoloaded = true;
            if std::path::Path::new(&self.model_path).exists() && self.whisper.is_none() {
                self.load_model();
            }
        }

        if let Some(dl) = &self.download {
            if dl.done.load(Ordering::Relaxed) {
                let err = dl.error.lock().ok().and_then(|mut g| g.take());
                self.download = None;
                if let Some(e) = err {
                    self.status = format!("Download failed: {e}");
                } else {
                    self.status = "Model downloaded. Loading…".to_string();
                    self.load_model();
                }
            }
        }

        // Handle a pending view switch (resizes window + sets always-on-top before drawing this frame's contents)
        if let Some(target) = self.pending_view_switch.take() {
            self.view = target;
            match target {
                ViewMode::Compact => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                        360.0, 220.0,
                    )));
                    ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                        if self.compact_always_on_top {
                            egui::WindowLevel::AlwaysOnTop
                        } else {
                            egui::WindowLevel::Normal
                        },
                    ));
                }
                _ => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                        900.0, 700.0,
                    )));
                    ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                        egui::WindowLevel::Normal,
                    ));
                }
            }
        }

        if self.view == ViewMode::Compact {
            self.render_compact(ui);
            if self.recording {
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
            }
            return;
        }

        {
            // Header with brand + view tabs
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("anuvaad")
                        .size(22.0)
                        .strong()
                        .color(ACCENT),
                );
                ui.label(
                    egui::RichText::new("· record · transcribe · translate")
                        .size(13.0)
                        .color(egui::Color32::from_gray(140)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button("⊟ Compact")
                        .on_hover_text("Shrink to a tiny floating window")
                        .clicked()
                    {
                        self.pending_view_switch = Some(ViewMode::Compact);
                    }
                    let mut view = self.view;
                    ui.selectable_value(&mut view, ViewMode::Sessions, "📂 Sessions");
                    ui.selectable_value(&mut view, ViewMode::Record, "🎙 Record");
                    if view != self.view {
                        self.view = view;
                    }
                });
            });
            ui.add_space(4.0);
            ui.separator();
            ui.add_space(6.0);

            if self.view == ViewMode::Sessions {
                self.render_sessions(ui);
                return;
            }

            ui.horizontal(|ui| {
                ui.label("Microphone:");
                let cur = self
                    .mic_devices
                    .get(self.mic_idx)
                    .map(|(n, _)| n.as_str())
                    .unwrap_or("<none>");
                egui::ComboBox::from_id_salt("mic")
                    .selected_text(cur)
                    .width(380.0)
                    .show_ui(ui, |ui| {
                        for (i, (n, _)) in self.mic_devices.iter().enumerate() {
                            ui.selectable_value(&mut self.mic_idx, i, n);
                        }
                    });
            });
            ui.horizontal(|ui| {
                ui.label("System output (loopback):");
                let cur = self
                    .out_devices
                    .get(self.out_idx)
                    .map(|(n, _)| n.as_str())
                    .unwrap_or("<none>");
                egui::ComboBox::from_id_salt("out")
                    .selected_text(cur)
                    .width(380.0)
                    .show_ui(ui, |ui| {
                        for (i, (n, _)) in self.out_devices.iter().enumerate() {
                            ui.selectable_value(&mut self.out_idx, i, n);
                        }
                    });
            });
            ui.horizontal(|ui| {
                ui.label("Output folder:");
                ui.add(egui::TextEdit::singleline(&mut self.output_dir).desired_width(420.0));
            });

            ui.horizontal(|ui| {
                ui.label("Audio language:");
                let mut cfg = self.translation_cfg.lock().unwrap();
                // The selected entry is either Auto (when whisper_lang is None) or matches whisper code.
                let current_idx = if cfg.whisper_lang.is_none() {
                    0
                } else {
                    LANGUAGES
                        .iter()
                        .position(|l| {
                            !l.whisper.is_empty()
                                && Some(l.whisper) == cfg.whisper_lang.as_deref()
                                && l.nllb == cfg.src
                        })
                        .or_else(|| {
                            LANGUAGES
                                .iter()
                                .position(|l| Some(l.whisper) == cfg.whisper_lang.as_deref())
                        })
                        .unwrap_or(0)
                };
                let current_label = LANGUAGES[current_idx].name;
                egui::ComboBox::from_id_salt("audiolang")
                    .selected_text(current_label)
                    .width(220.0)
                    .show_ui(ui, |ui| {
                        for (i, l) in LANGUAGES.iter().enumerate() {
                            if ui.selectable_label(i == current_idx, l.name).clicked() {
                                if l.whisper.is_empty() {
                                    cfg.whisper_lang = None;
                                } else {
                                    cfg.whisper_lang = Some(l.whisper.to_string());
                                }
                                cfg.src = l.nllb.to_string();
                            }
                        }
                    });
                drop(cfg);

                ui.label("·");
                ui.label("Transcribe:");
                let cur = self.transcribe_mode.label();
                egui::ComboBox::from_id_salt("xmode")
                    .selected_text(cur)
                    .width(180.0)
                    .show_ui(ui, |ui| {
                        for m in [
                            TranscribeMode::Off,
                            TranscribeMode::MicOnly,
                            TranscribeMode::SystemOnly,
                            TranscribeMode::Both,
                        ] {
                            ui.selectable_value(&mut self.transcribe_mode, m, m.label());
                        }
                    });
            });

            ui.horizontal(|ui| {
                let translator_ready = self
                    .translator
                    .lock()
                    .ok()
                    .map(|g| g.is_some())
                    .unwrap_or(false);
                let loading = *self.translator_loading.lock().unwrap();

                let mut cfg = self.translation_cfg.lock().unwrap();
                let prev_enabled = cfg.enabled;
                ui.checkbox(&mut cfg.enabled, "Translate");

                let tgt_label = translator::find_by_nllb(&cfg.tgt)
                    .map(|l| l.name)
                    .unwrap_or("?");
                egui::ComboBox::from_id_salt("trtgt")
                    .selected_text(format!("→ {tgt_label}"))
                    .width(200.0)
                    .show_ui(ui, |ui| {
                        for l in LANGUAGES.iter().filter(|l| !l.whisper.is_empty()) {
                            if ui.selectable_label(cfg.tgt == l.nllb, l.name).clicked() {
                                cfg.tgt = l.nllb.to_string();
                            }
                        }
                    });

                if cfg.enabled && cfg.whisper_lang.is_none() {
                    ui.colored_label(
                        egui::Color32::from_rgb(220, 180, 100),
                        "⚠ pick an audio language",
                    );
                }

                drop(cfg);

                if translator_ready {
                    ui.colored_label(egui::Color32::from_rgb(80, 200, 120), "✔ NLLB ready");
                } else if loading {
                    ui.weak("loading…");
                } else if ui.button("Init translator").clicked() {
                    self.start_translator();
                }

                let enabled_now = self.translation_cfg.lock().unwrap().enabled;
                if enabled_now && !prev_enabled && !translator_ready && !loading {
                    self.start_translator();
                }
            });
            ui.horizontal(|ui| {
                ui.weak(self.translator_status.lock().unwrap().as_str());
            });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.label("Whisper model:");
                let current_size_idx = model::MODEL_SIZES
                    .iter()
                    .position(|s| self.model_path == model::model_path_for(s.key).to_string_lossy())
                    .unwrap_or(1);
                let label = model::MODEL_SIZES[current_size_idx].display;
                egui::ComboBox::from_id_salt("modelsize")
                    .selected_text(label)
                    .width(360.0)
                    .show_ui(ui, |ui| {
                        for (i, s) in model::MODEL_SIZES.iter().enumerate() {
                            if ui
                                .selectable_label(i == current_size_idx, s.display)
                                .clicked()
                            {
                                let new_path =
                                    model::model_path_for(s.key).to_string_lossy().into_owned();
                                if new_path != self.model_path {
                                    self.model_path = new_path;
                                    self.whisper = None;
                                    self.whisper_path_loaded = None;
                                    self.autoloaded = false;
                                }
                            }
                        }
                    });
                if self.model_loaded() {
                    ui.colored_label(egui::Color32::from_rgb(80, 200, 120), "✔ loaded");
                } else if ui.button("Load").clicked() {
                    self.load_model();
                }
            });
            ui.horizontal(|ui| {
                ui.label("Path:");
                ui.add(egui::TextEdit::singleline(&mut self.model_path).desired_width(420.0));
            });
            ui.horizontal(|ui| {
                let exists = std::path::Path::new(&self.model_path).exists();
                let cur_size = model::MODEL_SIZES
                    .iter()
                    .find(|s| self.model_path == model::model_path_for(s.key).to_string_lossy())
                    .map(|s| (s.key, s.approx_mb));
                if !exists && self.download.is_none() {
                    let (lbl, url) = match cur_size {
                        Some((key, mb)) => (
                            format!("⬇ Download {key} model (~{mb} MB)"),
                            model::model_url_for(key),
                        ),
                        None => (
                            "⬇ Download base model (~140 MB)".to_string(),
                            model::MODEL_URL.to_string(),
                        ),
                    };
                    if ui.button(lbl).clicked() {
                        self.start_download_url(url);
                    }
                }
                if let Some(dl) = &self.download {
                    let cur = dl.bytes.load(Ordering::Relaxed);
                    let total = dl.total.load(Ordering::Relaxed);
                    if total > 0 {
                        ui.add(
                            egui::ProgressBar::new(cur as f32 / total as f32)
                                .desired_width(360.0)
                                .text(format!(
                                    "{:.1} / {:.1} MB",
                                    cur as f64 / 1e6,
                                    total as f64 / 1e6
                                )),
                        );
                    } else {
                        ui.label(format!("Downloaded {:.1} MB", cur as f64 / 1e6));
                    }
                }
            });

            ui.add_space(10.0);

            ui.horizontal(|ui| {
                let label = if self.recording {
                    "⏹  Stop"
                } else {
                    "⏺  Record"
                };
                let btn = egui::Button::new(label).min_size(egui::vec2(120.0, 32.0));
                if ui.add(btn).clicked() {
                    if self.recording {
                        self.stop();
                    } else {
                        self.start();
                    }
                }
                if self.recording {
                    if let Some(t) = self.record_start {
                        let e = t.elapsed().as_secs();
                        ui.colored_label(
                            egui::Color32::from_rgb(220, 60, 60),
                            format!("● REC {:02}:{:02}", e / 60, e % 60),
                        );
                    }
                }
            });

            ui.add_space(8.0);
            ui.separator();
            ui.label(&self.status);
            ui.add_space(6.0);

            ui.label("Transcript:");
            egui::Frame::group(ui.style()).show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .min_scrolled_height(220.0)
                    .show(ui, |ui| {
                        let t = self.transcript.lock().unwrap();
                        let empty = t.committed.is_empty()
                            && t.provisional_mic.is_empty()
                            && t.provisional_sys.is_empty();
                        if empty {
                            ui.weak("(no speech transcribed yet)");
                        } else {
                            for line in &t.committed {
                                render_line(ui, line, false);
                            }
                            let mut prov: Vec<&TranscriptLine> = t
                                .provisional_mic
                                .iter()
                                .chain(t.provisional_sys.iter())
                                .collect();
                            prov.sort_by(|a, b| {
                                a.start_s
                                    .partial_cmp(&b.start_s)
                                    .unwrap_or(std::cmp::Ordering::Equal)
                            });
                            for line in prov {
                                render_line(ui, line, true);
                            }
                        }
                    });
            });
        }

        if self.recording || self.download.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }
}

fn session_card(ui: &mut egui::Ui, s: &SessionMeta, selected: bool) -> egui::Response {
    let fill = if selected {
        SURFACE_ALT
    } else {
        egui::Color32::TRANSPARENT
    };
    let frame = egui::Frame::none()
        .fill(fill)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .corner_radius(egui::CornerRadius::same(6));
    let r = frame
        .show(ui, |ui| {
            ui.vertical(|ui| {
                let title_color = if selected { ACCENT } else { TEXT };
                let title = s.label.clone().unwrap_or_else(|| s.id.clone());
                ui.label(
                    egui::RichText::new(&title)
                        .strong()
                        .color(title_color)
                        .size(13.0),
                );
                if s.label.is_some() {
                    ui.label(egui::RichText::new(&s.id).size(10.0).color(TEXT_DIM));
                }
                let mins = (s.duration_s / 60.0).floor() as u32;
                let secs = (s.duration_s % 60.0).round() as u32;
                let dur = if mins > 0 {
                    format!("{mins}m {secs}s")
                } else {
                    format!("{secs}s")
                };
                ui.horizontal(|ui| {
                    if s.has_mic {
                        ui.label(egui::RichText::new("MIC").size(10.0).color(MIC_COLOR));
                    }
                    if s.has_sys {
                        ui.label(egui::RichText::new("SYS").size(10.0).color(SYS_COLOR));
                    }
                    if s.translate_enabled {
                        ui.label(egui::RichText::new("TR").size(10.0).color(TRANS_COLOR));
                    }
                    ui.label(egui::RichText::new(dur).size(11.0).color(TEXT_DIM));
                });
            });
        })
        .response;
    let r = r.interact(egui::Sense::click());
    if r.hovered() && !selected {
        ui.painter().rect_filled(
            r.rect,
            6.0,
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 8),
        );
    }
    r
}

enum SessionAction {
    Rename(String),
}

impl AudioRecorder {
    fn apply_session_action(&mut self, action: SessionAction, session: &SessionMeta) {
        match action {
            SessionAction::Rename(new_label) => {
                let label_opt = if new_label.trim().is_empty() {
                    None
                } else {
                    Some(new_label.trim().to_string())
                };
                if let Err(e) = sessions::set_label(&session.dir, label_opt.as_deref()) {
                    self.status = format!("Rename failed: {e}");
                } else {
                    self.session_cache_dir.clear(); // trigger rescan
                    self.status = "Session renamed".to_string();
                }
                self.rename_target_id = None;
                self.rename_buffer.clear();
            }
        }
    }

    fn render_session_detail(
        &mut self,
        ui: &mut egui::Ui,
        s: &SessionMeta,
        transcript: Option<&str>,
    ) -> Option<SessionAction> {
        let mut action: Option<SessionAction> = None;
        ui.add_space(8.0);

        let editing = self.rename_target_id.as_deref() == Some(s.id.as_str());
        ui.horizontal(|ui| {
            if editing {
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.rename_buffer)
                        .desired_width(280.0)
                        .hint_text("session label"),
                );
                let submit = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if submit || ui.button("✓").clicked() {
                    action = Some(SessionAction::Rename(self.rename_buffer.clone()));
                }
                if ui.button("✕").clicked() {
                    self.rename_target_id = None;
                    self.rename_buffer.clear();
                }
            } else {
                let display = s.label.clone().unwrap_or_else(|| s.id.clone());
                ui.label(
                    egui::RichText::new(&display)
                        .strong()
                        .size(18.0)
                        .color(ACCENT),
                );
                if s.label.is_some() {
                    ui.label(
                        egui::RichText::new(format!("({})", s.id))
                            .size(11.0)
                            .color(TEXT_DIM),
                    );
                }
                if ui
                    .small_button("✏")
                    .on_hover_text("Rename session")
                    .clicked()
                {
                    self.rename_buffer = s.label.clone().unwrap_or_default();
                    self.rename_target_id = Some(s.id.clone());
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("📁 Folder").clicked() {
                    let _ = std::process::Command::new("explorer").arg(&s.dir).spawn();
                }
                if s.has_sys && ui.button("🔊 System").clicked() {
                    play_with_default(&s.sys_wav());
                }
                if s.has_mic && ui.button("🔊 Mic").clicked() {
                    play_with_default(&s.mic_wav());
                }
            });
        });
        ui.add_space(6.0);

        egui::Grid::new("meta")
            .num_columns(2)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                meta_row(ui, "Started", &s.started);
                meta_row(ui, "Duration", &format!("{:.1} s", s.duration_s));
                meta_row(ui, "Mode", &s.transcribe_mode);
                if let Some(wl) = &s.whisper_lang {
                    meta_row(ui, "Whisper language", wl);
                } else {
                    meta_row(ui, "Whisper language", "(auto-detect)");
                }
                meta_row(ui, "Mic device", &s.mic_device);
                meta_row(ui, "System device", &s.sys_device);
                if s.translate_enabled {
                    meta_row(
                        ui,
                        "Translation",
                        &format!("{} → {}", s.translate_src, s.translate_tgt),
                    );
                }
            });

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(6.0);
        ui.label(egui::RichText::new("Transcript").strong().color(TEXT_DIM));
        ui.add_space(4.0);

        egui::ScrollArea::vertical()
            .id_salt("transcript_view")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if let Some(text) = transcript {
                    if text.trim().is_empty() {
                        ui.weak("(empty transcript)");
                    } else {
                        for line in text.lines() {
                            let trimmed = line.trim_start();
                            if trimmed.starts_with("→") {
                                ui.label(
                                    egui::RichText::new(line)
                                        .italics()
                                        .color(TRANS_COLOR)
                                        .monospace(),
                                );
                            } else {
                                ui.monospace(line);
                            }
                        }
                    }
                } else {
                    ui.weak("(no transcript file in this session)");
                }
            });
        action
    }
}

fn meta_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(egui::RichText::new(label).color(TEXT_DIM).size(12.0));
    ui.label(egui::RichText::new(value).color(TEXT).size(12.0));
    ui.end_row();
}

fn play_with_default(path: &std::path::Path) {
    if let Some(p) = path.to_str() {
        let _ = std::process::Command::new("cmd")
            .args(["/c", "start", "", p])
            .spawn();
    }
}

fn render_line(ui: &mut egui::Ui, line: &TranscriptLine, provisional: bool) {
    let (tag_color, text_color) = match (line.source, provisional) {
        (Source::Mic, false) => (
            egui::Color32::from_rgb(120, 200, 255),
            egui::Color32::from_gray(220),
        ),
        (Source::Sys, false) => (
            egui::Color32::from_rgb(255, 200, 120),
            egui::Color32::from_gray(220),
        ),
        (Source::Mic, true) => (
            egui::Color32::from_rgb(70, 120, 160),
            egui::Color32::from_gray(140),
        ),
        (Source::Sys, true) => (
            egui::Color32::from_rgb(160, 130, 70),
            egui::Color32::from_gray(140),
        ),
    };
    ui.horizontal_wrapped(|ui| {
        ui.monospace(
            egui::RichText::new(format!("[{:>6.1}s]", line.start_s)).color(
                egui::Color32::from_gray(if provisional { 110 } else { 170 }),
            ),
        );
        ui.label(egui::RichText::new(format!("[{}]", line.source.tag())).color(tag_color));
        ui.label(egui::RichText::new(&line.text).color(text_color));
    });
    if let Some(tr) = &line.translation {
        ui.horizontal_wrapped(|ui| {
            ui.add_space(70.0);
            ui.label(
                egui::RichText::new(format!("→ {tr}"))
                    .italics()
                    .color(egui::Color32::from_rgb(160, 220, 180)),
            );
        });
    }
}

fn setup_fonts(ctx: &egui::Context) {
    use egui::FontData;
    let mut fonts = egui::FontDefinitions::default();

    // Best-effort fallback fonts from the Windows install.
    // Each is added to BOTH families so it can render glyphs the default font lacks.
    let candidates: &[(&str, &str, u32)] = &[
        ("nirmala", "C:\\Windows\\Fonts\\Nirmala.ttf", 0),
        ("nirmala_b", "C:\\Windows\\Fonts\\NirmalaB.ttf", 0),
        ("mangal", "C:\\Windows\\Fonts\\mangal.ttf", 0),
        ("latha", "C:\\Windows\\Fonts\\latha.ttf", 0),
        ("kartika", "C:\\Windows\\Fonts\\kartika.ttf", 0),
        ("yahei", "C:\\Windows\\Fonts\\msyh.ttc", 0),
        ("yugothic", "C:\\Windows\\Fonts\\YuGothR.ttc", 0),
        ("malgun", "C:\\Windows\\Fonts\\malgun.ttf", 0),
        ("emoji", "C:\\Windows\\Fonts\\seguiemj.ttf", 0),
    ];

    for (key, path, index) in candidates {
        if let Ok(data) = std::fs::read(path) {
            let mut font = FontData::from_owned(data);
            font.index = *index;
            fonts.font_data.insert((*key).to_string(), font.into());
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .push((*key).to_string());
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .push((*key).to_string());
        }
    }

    ctx.set_fonts(fonts);
}

fn apply_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = egui::Color32::from_rgb(22, 22, 28);
    visuals.window_fill = egui::Color32::from_rgb(28, 28, 36);
    visuals.extreme_bg_color = egui::Color32::from_rgb(16, 16, 22);
    visuals.selection.bg_fill = egui::Color32::from_rgb(90, 65, 165);
    visuals.selection.stroke.color = ACCENT;
    visuals.hyperlink_color = ACCENT;
    visuals.window_corner_radius = egui::CornerRadius::same(10);
    visuals.menu_corner_radius = egui::CornerRadius::same(8);
    visuals.widgets.noninteractive.corner_radius = egui::CornerRadius::same(6);
    visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(6);
    visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(6);
    visuals.widgets.active.corner_radius = egui::CornerRadius::same(6);
    visuals.widgets.open.corner_radius = egui::CornerRadius::same(6);
    visuals.widgets.inactive.bg_fill = SURFACE_ALT;
    visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(55, 55, 70);
    visuals.widgets.active.bg_fill = egui::Color32::from_rgb(70, 60, 110);
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    style.spacing.window_margin = egui::Margin::same(12);
    ctx.set_style(style);
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([900.0, 700.0])
            .with_min_inner_size([720.0, 560.0])
            .with_title("anuvaad"),
        ..Default::default()
    };
    eframe::run_native(
        "anuvaad",
        options,
        Box::new(|cc| {
            setup_fonts(&cc.egui_ctx);
            apply_theme(&cc.egui_ctx);
            Ok(Box::new(AudioRecorder::default()))
        }),
    )
}
