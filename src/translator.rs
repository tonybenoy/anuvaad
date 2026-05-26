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

/// A language available in both Whisper (2-letter code) and NLLB (BCP-47 + script).
pub struct Lang {
    pub whisper: &'static str,
    pub nllb: &'static str,
    pub name: &'static str,
}

/// First entry is treated as "auto-detect" for Whisper purposes (empty whisper code).
pub const LANGUAGES: &[Lang] = &[
    Lang {
        whisper: "",
        nllb: "eng_Latn",
        name: "Auto-detect",
    },
    Lang {
        whisper: "en",
        nllb: "eng_Latn",
        name: "English",
    },
    Lang {
        whisper: "hi",
        nllb: "hin_Deva",
        name: "Hindi",
    },
    Lang {
        whisper: "es",
        nllb: "spa_Latn",
        name: "Spanish",
    },
    Lang {
        whisper: "fr",
        nllb: "fra_Latn",
        name: "French",
    },
    Lang {
        whisper: "de",
        nllb: "deu_Latn",
        name: "German",
    },
    Lang {
        whisper: "it",
        nllb: "ita_Latn",
        name: "Italian",
    },
    Lang {
        whisper: "pt",
        nllb: "por_Latn",
        name: "Portuguese",
    },
    Lang {
        whisper: "ru",
        nllb: "rus_Cyrl",
        name: "Russian",
    },
    Lang {
        whisper: "zh",
        nllb: "zho_Hans",
        name: "Chinese (Simplified)",
    },
    Lang {
        whisper: "zh",
        nllb: "zho_Hant",
        name: "Chinese (Traditional)",
    },
    Lang {
        whisper: "ja",
        nllb: "jpn_Jpan",
        name: "Japanese",
    },
    Lang {
        whisper: "ko",
        nllb: "kor_Hang",
        name: "Korean",
    },
    Lang {
        whisper: "ar",
        nllb: "arb_Arab",
        name: "Arabic",
    },
    Lang {
        whisper: "bn",
        nllb: "ben_Beng",
        name: "Bengali",
    },
    Lang {
        whisper: "ta",
        nllb: "tam_Taml",
        name: "Tamil",
    },
    Lang {
        whisper: "te",
        nllb: "tel_Telu",
        name: "Telugu",
    },
    Lang {
        whisper: "mr",
        nllb: "mar_Deva",
        name: "Marathi",
    },
    Lang {
        whisper: "ur",
        nllb: "urd_Arab",
        name: "Urdu",
    },
    Lang {
        whisper: "pa",
        nllb: "pan_Guru",
        name: "Punjabi",
    },
    Lang {
        whisper: "gu",
        nllb: "guj_Gujr",
        name: "Gujarati",
    },
    Lang {
        whisper: "kn",
        nllb: "kan_Knda",
        name: "Kannada",
    },
    Lang {
        whisper: "ml",
        nllb: "mal_Mlym",
        name: "Malayalam",
    },
    Lang {
        whisper: "nl",
        nllb: "nld_Latn",
        name: "Dutch",
    },
    Lang {
        whisper: "pl",
        nllb: "pol_Latn",
        name: "Polish",
    },
    Lang {
        whisper: "tr",
        nllb: "tur_Latn",
        name: "Turkish",
    },
    Lang {
        whisper: "vi",
        nllb: "vie_Latn",
        name: "Vietnamese",
    },
    Lang {
        whisper: "id",
        nllb: "ind_Latn",
        name: "Indonesian",
    },
    Lang {
        whisper: "th",
        nllb: "tha_Thai",
        name: "Thai",
    },
    Lang {
        whisper: "el",
        nllb: "ell_Grek",
        name: "Greek",
    },
    Lang {
        whisper: "he",
        nllb: "heb_Hebr",
        name: "Hebrew",
    },
    Lang {
        whisper: "sw",
        nllb: "swh_Latn",
        name: "Swahili",
    },
];

pub fn find_by_nllb(code: &str) -> Option<&'static Lang> {
    LANGUAGES.iter().find(|l| l.nllb == code)
}
