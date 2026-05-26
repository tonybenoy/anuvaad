# Usage

## Window layout

```
┌──────────────────────────────────────────────────────────┐
│ anuvaad — recorder + live transcription                  │
│                                                          │
│ Microphone:               [ Realtek Audio       ▾]      │
│ System output (loopback): [ Speakers (Realtek)  ▾]      │
│ Output folder:            [ C:\Users\...\Music\anuvaad ] │
│ Transcribe:               [ Both              ▾]        │
│ ☐ Translate  [from English ▾] [→ Hindi ▾] [Init…]       │
│                                                          │
│ ───────────────────────────────────────────────────      │
│ Model: [ ...\ggml-base.bin ]   ✔ loaded                 │
│                                                          │
│ [ ⏺ Record ]   ● REC 00:23                              │
│                                                          │
│ ──── Transcript ─────────────────────────────────        │
│ [   0.0s] [SYS] Welcome to the show.                    │
│              → आपका शो में स्वागत है।                     │
│ [   3.4s] [MIC] Thanks for having me.                   │
│              → मुझे आमंत्रित करने के लिए धन्यवाद।          │
│ [   6.1s] [SYS] Let's get started.   ← gray = provisional│
└──────────────────────────────────────────────────────────┘
```

## Picking devices

- **Microphone** — any active input device. Defaults to your Windows default mic.
- **System output (loopback)** — the *output* device whose audio you want to capture (typically your speakers or headphones). Anuvaad opens it in WASAPI loopback mode so it records exactly what those speakers would play.

Switching defaults mid-Windows-session may require restarting anuvaad to refresh the dropdown.

## Transcribe modes

| Mode | Behavior |
|---|---|
| Off | Just record, no live text |
| Mic only | Transcribe mic; ignore system audio for text (but still record both WAVs) |
| System only | Transcribe system audio only (default — good for meetings, lectures) |
| Both | Both streams independently transcribed, tagged `[MIC]` and `[SYS]` |

## Provisional vs. committed text

- **Committed** (full color): older than ~2 seconds, won't change. Saved to the transcript file.
- **Provisional** (gray): most recent ~2 seconds. May rewrite itself as more audio arrives and Whisper gets more context.

On **Stop**, all provisional text is committed and the transcript is written to disk.

## Translation

Once you've initialized the translator (one-time setup, see [Translation setup](translation.md)):

1. Pick a source language (the language being *spoken*).
2. Pick a target language.
3. Toggle **Translate** on.

Each committed line gets a translation appended underneath (`→ ...` in italic green). Provisional lines are not translated; only stable text is sent to NLLB.

The `.txt` output also includes the translations.

## File naming

```
%USERPROFILE%\Music\anuvaad\
└── 20260101-153045-mic.wav         (original mic, 16-bit PCM)
└── 20260101-153045-system.wav      (loopback, 16-bit PCM)
└── 20260101-153045-transcript.txt  (transcript + translations)
```
