use crate::transcribe::{LiveTranscript, TranslationRequest};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

pub struct Translator {
    child: Mutex<Option<Child>>,
    request_tx: Sender<TranslationRequest>,
    shutdown: Arc<AtomicBool>,
    pub device: String,
}

impl Translator {
    pub fn spawn(
        python: &str,
        script: &Path,
        transcript: Arc<Mutex<LiveTranscript>>,
    ) -> Result<Self, String> {
        let mut child = Command::new(python)
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("spawn {python}: {e}"))?;

        let stdin = child.stdin.take().ok_or_else(|| "no stdin".to_string())?;
        let stdout = child.stdout.take().ok_or_else(|| "no stdout".to_string())?;
        let stderr = child.stderr.take();
        let mut reader = BufReader::new(stdout);

        // Forward stderr to our stderr for visibility.
        if let Some(stderr) = stderr {
            std::thread::spawn(move || {
                let r = BufReader::new(stderr);
                for line in r.lines().flatten() {
                    eprintln!("{line}");
                }
            });
        }

        let mut ready_line = String::new();
        reader
            .read_line(&mut ready_line)
            .map_err(|e| format!("read ready: {e}"))?;
        let trimmed = ready_line.trim();
        let ready: serde_json::Value =
            serde_json::from_str(trimmed).map_err(|e| format!("parse ready ({trimmed:?}): {e}"))?;
        if let Some(err) = ready.get("error").and_then(|v| v.as_str()) {
            return Err(err.to_string());
        }
        if !ready
            .get("ready")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return Err(format!("translator not ready: {trimmed}"));
        }
        let device = ready
            .get("device")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let (request_tx, request_rx) = mpsc::channel::<TranslationRequest>();
        let shutdown = Arc::new(AtomicBool::new(false));

        // Writer thread: send JSON requests to Python stdin
        let shutdown_w = shutdown.clone();
        let mut stdin_w = stdin;
        std::thread::spawn(move || {
            while let Ok(req) = request_rx.recv() {
                if shutdown_w.load(Ordering::Relaxed) {
                    break;
                }
                let json = serde_json::json!({
                    "id": req.line_id,
                    "text": req.text,
                    "src": req.src,
                    "tgt": req.tgt,
                });
                if writeln!(stdin_w, "{json}").is_err() {
                    break;
                }
                let _ = stdin_w.flush();
            }
        });

        // Reader thread: parse responses and patch transcript lines
        let shutdown_r = shutdown.clone();
        let transcript_r = transcript;
        std::thread::spawn(move || {
            let mut line = String::new();
            loop {
                line.clear();
                let n = match reader.read_line(&mut line) {
                    Ok(n) => n,
                    Err(_) => break,
                };
                if n == 0 || shutdown_r.load(Ordering::Relaxed) {
                    break;
                }
                let trimmed = line.trim();
                let Ok(resp) = serde_json::from_str::<serde_json::Value>(trimmed) else {
                    continue;
                };
                let id = resp.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                if let Some(trans) = resp.get("translation").and_then(|v| v.as_str()) {
                    if let Ok(mut t) = transcript_r.lock() {
                        for l in t.committed.iter_mut() {
                            if l.id == id {
                                l.translation = Some(trans.to_string());
                                break;
                            }
                        }
                    }
                }
            }
        });

        Ok(Self {
            child: Mutex::new(Some(child)),
            request_tx,
            shutdown,
            device,
        })
    }

    pub fn request(&self, req: TranslationRequest) {
        let _ = self.request_tx.send(req);
    }

    pub fn sender(&self) -> Sender<TranslationRequest> {
        self.request_tx.clone()
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Ok(mut c) = self.child.lock() {
            if let Some(mut child) = c.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

impl Drop for Translator {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// NLLB language code (BCP-47 + script) and a friendly display name.
pub const LANGUAGES: &[(&str, &str)] = &[
    ("eng_Latn", "English"),
    ("hin_Deva", "Hindi"),
    ("spa_Latn", "Spanish"),
    ("fra_Latn", "French"),
    ("deu_Latn", "German"),
    ("ita_Latn", "Italian"),
    ("por_Latn", "Portuguese"),
    ("rus_Cyrl", "Russian"),
    ("zho_Hans", "Chinese (Simplified)"),
    ("zho_Hant", "Chinese (Traditional)"),
    ("jpn_Jpan", "Japanese"),
    ("kor_Hang", "Korean"),
    ("arb_Arab", "Arabic"),
    ("ben_Beng", "Bengali"),
    ("tam_Taml", "Tamil"),
    ("tel_Telu", "Telugu"),
    ("mar_Deva", "Marathi"),
    ("urd_Arab", "Urdu"),
    ("pan_Guru", "Punjabi"),
    ("guj_Gujr", "Gujarati"),
    ("kan_Knda", "Kannada"),
    ("mal_Mlym", "Malayalam"),
    ("nld_Latn", "Dutch"),
    ("pol_Latn", "Polish"),
    ("tur_Latn", "Turkish"),
    ("vie_Latn", "Vietnamese"),
    ("ind_Latn", "Indonesian"),
    ("tha_Thai", "Thai"),
    ("ell_Grek", "Greek"),
    ("heb_Hebr", "Hebrew"),
    ("swh_Latn", "Swahili"),
];
