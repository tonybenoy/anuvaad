use std::fs::{create_dir_all, File};
use std::io::{BufWriter, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

pub struct ModelSize {
    pub key: &'static str,
    pub display: &'static str,
    pub approx_mb: u32,
}

pub const MODEL_SIZES: &[ModelSize] = &[
    ModelSize {
        key: "tiny",
        display: "tiny (75 MB) — fastest",
        approx_mb: 75,
    },
    ModelSize {
        key: "base",
        display: "base (142 MB) — default",
        approx_mb: 142,
    },
    ModelSize {
        key: "small",
        display: "small (466 MB) — recommended for non-English",
        approx_mb: 466,
    },
    ModelSize {
        key: "medium",
        display: "medium (1.5 GB) — high accuracy",
        approx_mb: 1500,
    },
    ModelSize {
        key: "large-v3",
        display: "large-v3 (3 GB) — best",
        approx_mb: 3000,
    },
];

pub fn models_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("anuvaad")
        .join("models")
}

pub fn model_path_for(size_key: &str) -> PathBuf {
    models_dir().join(format!("ggml-{size_key}.bin"))
}

pub fn model_url_for(size_key: &str) -> String {
    format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{size_key}.bin")
}

/// Back-compat helpers — pointed at the "base" size.
pub const MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin";

pub fn default_model_path() -> PathBuf {
    model_path_for("base")
}

pub struct DownloadHandle {
    pub bytes: Arc<AtomicU64>,
    pub total: Arc<AtomicU64>,
    pub done: Arc<AtomicBool>,
    pub error: Arc<Mutex<Option<String>>>,
    pub _handle: JoinHandle<()>,
}

pub fn download_model_async(dest: PathBuf, url: String) -> DownloadHandle {
    let bytes = Arc::new(AtomicU64::new(0));
    let total = Arc::new(AtomicU64::new(0));
    let done = Arc::new(AtomicBool::new(false));
    let error = Arc::new(Mutex::new(None));

    let b = bytes.clone();
    let t = total.clone();
    let d = done.clone();
    let e = error.clone();

    let handle = std::thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            if let Some(parent) = dest.parent() {
                create_dir_all(parent).map_err(|x| format!("mkdir: {x}"))?;
            }
            let tmp = dest.with_extension("bin.part");
            let resp = ureq::get(&url).call().map_err(|x| format!("get: {x}"))?;
            if let Some(cl) = resp
                .headers()
                .get("Content-Length")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
            {
                t.store(cl, Ordering::Relaxed);
            }
            let mut body = resp.into_body();
            let mut reader = body.as_reader();
            let mut file = BufWriter::new(File::create(&tmp).map_err(|x| format!("create: {x}"))?);
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                let n = reader.read(&mut buf).map_err(|x| format!("read: {x}"))?;
                if n == 0 {
                    break;
                }
                file.write_all(&buf[..n])
                    .map_err(|x| format!("write: {x}"))?;
                b.fetch_add(n as u64, Ordering::Relaxed);
            }
            file.into_inner()
                .map_err(|x| format!("flush: {x}"))?
                .sync_all()
                .map_err(|x| format!("sync: {x}"))?;
            if dest.exists() {
                let _ = std::fs::remove_file(&dest);
            }
            std::fs::rename(&tmp, &dest).map_err(|x| format!("rename: {x}"))?;
            Ok(())
        })();
        if let Err(msg) = result {
            if let Ok(mut g) = e.lock() {
                *g = Some(msg);
            }
        }
        d.store(true, Ordering::Relaxed);
    });

    DownloadHandle {
        bytes,
        total,
        done,
        error,
        _handle: handle,
    }
}
