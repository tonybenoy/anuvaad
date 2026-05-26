# Installation

Two paths: download a prebuilt binary (recommended) or build from source.

## Prebuilt binaries

Grab the latest from [Releases](https://github.com/tonybenoy/anuvaad/releases).

Two variants are published per release:

| File | When to choose | Size |
|---|---|---|
| `anuvaad-cuda.exe` | You have an NVIDIA GPU and want ~10–20× faster transcription | ~38 MB |
| `anuvaad-cpu.exe` | You don't have CUDA, or want a smaller binary | ~10 MB |

`anuvaad-cuda.exe` will fall back to CPU at runtime if no compatible GPU is found, so when in doubt pick the CUDA build.

### NVIDIA driver requirement (CUDA build only)

The CUDA build needs a recent driver. Anything from the last year or two works. Test with:

```powershell
nvidia-smi
```

If that prints your GPU and a driver version, you're set. You do **not** need to install the CUDA Toolkit just to run the prebuilt binary — only to rebuild from source.

## First run

1. Double-click `anuvaad-*.exe`.
2. Click **Download base model (~140 MB)**. The Whisper model lands in `%LOCALAPPDATA%\anuvaad\models\ggml-base.bin`.
3. Pick a microphone and an output device.
4. Click **⏺ Record**.
5. Click **⏹ Stop** when done.

Three files are saved per session under `%USERPROFILE%\Music\anuvaad\`:

- `<timestamp>-mic.wav`
- `<timestamp>-system.wav`
- `<timestamp>-transcript.txt`

## Translation (optional)

See [Translation setup](translation.md). You'll need Python 3.10+ and a one-time `pip install`.
