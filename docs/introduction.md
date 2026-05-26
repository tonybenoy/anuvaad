# anuvaad

**anuvaad** (Hindi: अनुवाद, "translation") is a Windows desktop app that records, transcribes, and translates audio in real time, all locally.

## What it does

- **Records** your microphone and what your speakers are playing, simultaneously, into two WAV files per session.
- **Transcribes** both streams live using [whisper.cpp](https://github.com/ggerganov/whisper.cpp). Words appear in the GUI within ~1–2 seconds.
- **Translates** committed transcript lines between any of NLLB-200's ~200 languages, on demand.

## Why two streams?

For meetings, interviews, podcasts, and language practice you often want to keep "what you said" separate from "what they said". Most loopback tools mix the two; anuvaad keeps them addressable.

## What's local vs. what isn't

| Component | Local? | Notes |
|---|---|---|
| Audio capture | ✅ Yes | Native WASAPI via cpal |
| Whisper transcription | ✅ Yes | whisper-rs, CPU or CUDA |
| NLLB translation | ✅ Yes | Python + CTranslate2 |
| First-time model downloads | 🌐 Hugging Face | Whisper ~140 MB, NLLB ~600 MB |

Nothing leaves your machine at runtime.

## Status

Active prototype. Single-developer project. Expect rough edges.

See [Installation](install.md) to get started.
