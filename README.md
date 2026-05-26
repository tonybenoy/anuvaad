# anuvaad

Real-time audio recorder, transcriber, and translator for Windows.

- Captures your **microphone** and **system audio** (WASAPI loopback) simultaneously, saving each as a WAV.
- Transcribes both streams **live** with [whisper.cpp](https://github.com/ggerganov/whisper.cpp) (whisper-rs) — accelerated on NVIDIA GPUs.
- Optionally **translates** committed lines between any of NLLB-200's ~200 languages via a Python subprocess (CTranslate2).
- Sliding-window UI: provisional text appears in gray while audio is still arriving, then commits to a stable transcript.

Documentation: **https://tonybenoy.github.io/anuvaad/**

## Quick start

1. Grab the latest build for your setup from [Releases](https://github.com/tonybenoy/anuvaad/releases):
   - `anuvaad-cuda.exe` — uses your NVIDIA GPU for ~10–20× faster transcription. Requires a recent NVIDIA driver.
   - `anuvaad-cpu.exe` — pure CPU, works on any modern machine.
2. Double-click. On first run, click **Download base model (~140 MB)** to fetch the Whisper model.
3. Pick mic + output device, click **Record**.

For translation, install Python 3.10+ and run:

```powershell
python -m pip install ctranslate2 transformers sentencepiece huggingface_hub
```

The first time you toggle Translate on, the NLLB-200 model (~600 MB) is downloaded automatically.

## Features

| | |
|---|---|
| **Mic capture** | Any input device, original channel count and sample rate, saved as 16-bit PCM WAV |
| **System audio** | WASAPI loopback on the selected output device — captures whatever your speakers play |
| **Transcription** | Whisper base by default (model path configurable); 99 source languages auto-detected |
| **Streaming UX** | 1.2s update step, 5s rolling window, 2s provisional buffer |
| **Translation** | NLLB-200 distilled 600M (int8), ~200 languages, runs locally via CTranslate2 |
| **GPU** | Optional CUDA feature flag — falls back to CPU at runtime if no compatible GPU |

## Building from source

See [docs/build-from-source.md](docs/build-from-source.md) or the [online docs](https://tonybenoy.github.io/anuvaad/build-from-source.html).

Quick version (Windows):

```powershell
# Prereqs (one-time, all via winget):
winget install Microsoft.VisualStudio.BuildTools --override "--quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
winget install LLVM.LLVM
winget install Kitware.CMake
winget install Nvidia.CUDA   # optional, for --features cuda

# Build:
$env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin"
cargo build --release                  # CPU
cargo build --release --features cuda  # GPU
```

## Layout

```
src/
├── main.rs        GUI (egui/eframe) + glue
├── audio.rs       cpal capture, WAV writing, rolling PCM buffer
├── transcribe.rs  whisper-rs worker, sliding-window VAD, commit/provisional split
├── translator.rs  Python subprocess management + JSON protocol
└── model.rs       Whisper model auto-download
scripts/
└── translator.py  NLLB-200 CTranslate2 worker (reads stdin / writes stdout)
docs/             mdBook source for https://tonybenoy.github.io/anuvaad/
```

## Credits

- [whisper.cpp](https://github.com/ggerganov/whisper.cpp) — speech recognition
- [cpal](https://github.com/RustAudio/cpal) — cross-platform audio I/O
- [CTranslate2](https://github.com/OpenNMT/CTranslate2) — fast inference for the NLLB translation model
- [NLLB-200](https://ai.meta.com/research/no-language-left-behind/) — multilingual translation by Meta AI
- [egui / eframe](https://github.com/emilk/egui) — GUI

## License

MIT — see [LICENSE](LICENSE).
