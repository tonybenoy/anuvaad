# Architecture

A high-level pass through the threading model and data flow.

## Threads

```
┌─────────────┐
│ GUI (main)  │  egui/eframe — handles UI, owns the cpal Stream handles
└──┬──────────┘   (Stream is !Send so it must live on this thread)
   │ Arc<Mutex<LiveTranscript>>
   │ Arc<Mutex<TranslationConfig>>
   ▼
┌─────────────────────────────────────────────┐
│ Audio callback × 2  (cpal-owned)            │
│ — mic device input stream                   │
│ — output device input stream (WASAPI loopback)
│ Pushes f32 mono samples into Capture buffer │
│ and writes 16-bit PCM frames to WAV         │
└──┬──────────────────────────────────────────┘
   │ Arc<Mutex<CaptureState>>
   ▼
┌─────────────────────────┐
│ Transcribe worker       │  Wakes every ~1.2s
│ For each capture:       │  Peeks last 5s, runs Whisper,
│   peek → resample 16k   │  classifies segments as committed/provisional,
│   → whisper.full        │  pushes onto LiveTranscript
│   → classify segments   │
└──┬──────────────────────┘
   │ mpsc::Sender<TranslationRequest>
   ▼
┌─────────────────────────┐    stdin (JSON lines)
│ Translator writer thread├─────────────────┐
└─────────────────────────┘                 ▼
                                  ┌──────────────────────┐
                                  │ python translator.py │
                                  │ (CTranslate2 +       │
                                  │  NLLB-200)           │
                                  └──┬───────────────────┘
                                     │ stdout (JSON lines)
┌─────────────────────────┐          ▼
│ Translator reader thread│◀─────────┘
│ patches LiveTranscript  │
└─────────────────────────┘
```

## Data structures

### `CaptureState` (`audio.rs`)

One per source, behind a `Mutex`. The audio callback (running on a cpal-owned thread) pushes; the transcribe worker peeks.

```rust
pub struct CaptureState {
    pub wav: Option<WavWriter<BufWriter<File>>>, // currently-open WAV
    pub pcm: Vec<f32>,                            // rolling mono buffer
    pub total_pushed: u64,                        // for timestamp math
    pub channels: u16,
    pub sample_rate: u32,
}
```

- The buffer is capped at `MAX_BUFFER_SECS` (8s); excess is dropped from the front in the callback.
- `total_pushed` keeps growing forever, even as old samples age out. The combination tells us the absolute time of any sample.

### `LiveTranscript` (`transcribe.rs`)

Shared between the transcribe worker (writer), the translator reader thread (patcher), and the GUI (reader).

```rust
pub struct LiveTranscript {
    pub committed: Vec<TranscriptLine>,           // append-only, by time
    pub provisional_mic: Vec<TranscriptLine>,     // replaced each pass
    pub provisional_sys: Vec<TranscriptLine>,
    pub next_id: u64,                             // for translation routing
}

pub struct TranscriptLine {
    pub id: u64,
    pub start_s: f64,
    pub end_s: f64,
    pub source: Source,
    pub text: String,
    pub translation: Option<String>,
}
```

## Sliding-window logic

Every `STEP_MS` (1200 ms) for each source:

1. Peek the last `WINDOW_SECS` (5 s) of audio from `CaptureState`.
2. Resample to 16 kHz (whisper's native rate) using linear interpolation.
3. Skip if RMS is below `VOICE_RMS` (silence — cheap energy check).
4. Run `WhisperState::full` with greedy decoding.
5. For each segment Whisper produces:
   - Compute absolute time = window start + segment timestamp.
   - If the segment ends **before** `window_end - PROVISIONAL_SECS` (2 s): commit it.
   - Otherwise: it's provisional, may be revised next pass.
6. Track `committed_until_s` per source so re-transcribed audio (the rolling window overlaps) doesn't double-emit.

The provisional window is what makes the UX feel "live" — text shows up immediately and refines itself.

## Translation routing

Each committed line gets a monotonically increasing `id`. When committing:

1. The transcribe worker pushes the line onto `LiveTranscript.committed`.
2. If translation is enabled, it sends a `TranslationRequest { line_id, text, src, tgt }` to the translator over an mpsc channel.

The translator's **writer thread** serializes requests to JSON on the Python process's stdin. Its **reader thread** parses responses; on a hit it locks `LiveTranscript`, finds the line by `id`, and sets `translation = Some(...)`. The GUI sees the update on its next repaint.

## Why not gRPC / a real IPC framework?

NDJSON over pipes is two threads of trivial code and zero dependencies. The Python side gets ~1 ms of overhead per round-trip, dominated by inference (30+ ms on GPU). Not worth a framework.

## File layout

```
src/
├── main.rs        eframe::App impl, all GUI, owns cpal Streams, spawns workers
├── audio.rs       cpal stream construction, WAV writer, rolling PCM buffer
├── transcribe.rs  spawn_worker, sliding window, VAD, segment classification
├── translator.rs  Translator struct + LANGUAGES table + writer/reader threads
└── model.rs       Whisper model URL + async download with progress
scripts/
└── translator.py  Python NLLB worker (CTranslate2)
```
