use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct SessionMeta {
    pub id: String,
    pub label: Option<String>,
    pub dir: PathBuf,
    pub started: String,
    pub duration_s: f64,
    pub mic_device: String,
    pub sys_device: String,
    pub transcribe_mode: String,
    pub whisper_lang: Option<String>,
    pub translate_enabled: bool,
    pub translate_src: String,
    pub translate_tgt: String,
    pub has_mic: bool,
    pub has_sys: bool,
    pub has_transcript: bool,
}

impl SessionMeta {
    pub fn mic_wav(&self) -> PathBuf {
        self.dir.join("mic.wav")
    }
    pub fn sys_wav(&self) -> PathBuf {
        self.dir.join("system.wav")
    }
    pub fn transcript_path(&self) -> PathBuf {
        self.dir.join("transcript.txt")
    }
}

#[allow(clippy::too_many_arguments)]
pub fn write_metadata(
    session_dir: &Path,
    id: &str,
    label: Option<&str>,
    started_iso: &str,
    duration_s: f64,
    mic_device: &str,
    sys_device: &str,
    transcribe_mode: &str,
    whisper_lang: Option<&str>,
    translate_enabled: bool,
    translate_src: &str,
    translate_tgt: &str,
) -> std::io::Result<()> {
    let json = serde_json::json!({
        "id": id,
        "label": label,
        "started": started_iso,
        "duration_s": duration_s,
        "mic_device": mic_device,
        "sys_device": sys_device,
        "transcribe_mode": transcribe_mode,
        "whisper_lang": whisper_lang,
        "translation": {
            "enabled": translate_enabled,
            "src": translate_src,
            "tgt": translate_tgt
        }
    });
    fs::write(
        session_dir.join("session.json"),
        serde_json::to_string_pretty(&json)?,
    )
}

/// Update only the label field in an existing session.json (preserves other fields).
pub fn set_label(session_dir: &Path, label: Option<&str>) -> std::io::Result<()> {
    let path = session_dir.join("session.json");
    let mut json: serde_json::Value = fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(obj) = json.as_object_mut() {
        match label {
            Some(l) if !l.is_empty() => {
                obj.insert("label".into(), serde_json::Value::String(l.into()));
            }
            _ => {
                obj.remove("label");
            }
        }
    }
    fs::write(path, serde_json::to_string_pretty(&json)?)
}

pub fn discover(root: &Path) -> Vec<SessionMeta> {
    let Ok(read) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in read.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let id = entry.file_name().to_string_lossy().into_owned();
        if id.starts_with('.') {
            continue;
        }
        let meta_path = path.join("session.json");
        let mic_wav = path.join("mic.wav").exists();
        let sys_wav = path.join("system.wav").exists();
        let transcript = path.join("transcript.txt").exists();

        // Skip directories that don't look like sessions at all
        if !mic_wav && !sys_wav && !transcript && !meta_path.exists() {
            continue;
        }

        let meta_json: Option<serde_json::Value> = fs::read_to_string(&meta_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok());

        let started = meta_json
            .as_ref()
            .and_then(|v| v.get("started").and_then(|x| x.as_str()))
            .map(String::from)
            .unwrap_or_else(|| id.clone());
        let duration_s = meta_json
            .as_ref()
            .and_then(|v| v.get("duration_s").and_then(|x| x.as_f64()))
            .unwrap_or(0.0);
        let mic_device = meta_json
            .as_ref()
            .and_then(|v| v.get("mic_device").and_then(|x| x.as_str()))
            .unwrap_or("?")
            .to_string();
        let sys_device = meta_json
            .as_ref()
            .and_then(|v| v.get("sys_device").and_then(|x| x.as_str()))
            .unwrap_or("?")
            .to_string();
        let transcribe_mode = meta_json
            .as_ref()
            .and_then(|v| v.get("transcribe_mode").and_then(|x| x.as_str()))
            .unwrap_or("?")
            .to_string();
        let translation = meta_json.as_ref().and_then(|v| v.get("translation"));
        let translate_enabled = translation
            .and_then(|v| v.get("enabled").and_then(|x| x.as_bool()))
            .unwrap_or(false);
        let translate_src = translation
            .and_then(|v| v.get("src").and_then(|x| x.as_str()))
            .unwrap_or("")
            .to_string();
        let translate_tgt = translation
            .and_then(|v| v.get("tgt").and_then(|x| x.as_str()))
            .unwrap_or("")
            .to_string();
        let label = meta_json
            .as_ref()
            .and_then(|v| v.get("label").and_then(|x| x.as_str()))
            .filter(|s| !s.is_empty())
            .map(String::from);
        let whisper_lang = meta_json
            .as_ref()
            .and_then(|v| v.get("whisper_lang").and_then(|x| x.as_str()))
            .filter(|s| !s.is_empty())
            .map(String::from);

        out.push(SessionMeta {
            id,
            label,
            dir: path,
            started,
            duration_s,
            mic_device,
            sys_device,
            transcribe_mode,
            whisper_lang,
            translate_enabled,
            translate_src,
            translate_tgt,
            has_mic: mic_wav,
            has_sys: sys_wav,
            has_transcript: transcript,
        });
    }
    out.sort_by(|a, b| b.id.cmp(&a.id));
    out
}

pub fn read_transcript(session: &SessionMeta) -> Option<String> {
    fs::read_to_string(session.transcript_path()).ok()
}
