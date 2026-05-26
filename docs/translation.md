# Translation setup

Translation runs as a separate Python process bundled alongside the binary. It uses [CTranslate2](https://github.com/OpenNMT/CTranslate2) to run [NLLB-200-distilled-600M](https://huggingface.co/facebook/nllb-200-distilled-600M) — Meta's multilingual translation model that supports ~200 languages.

## Why Python?

Pure-Rust NLLB inference is workable but requires either writing the seq2seq generation loop on top of ONNX Runtime (~1000 lines of tensor code) or wrangling CTranslate2's Windows build (vcpkg + OpenBLAS + a CUDA version mismatch waiting to happen). The Python path takes a 1 ms IPC hit per translation and works reliably.

## One-time setup

### 1. Install Python 3.10+

```powershell
winget install Python.Python.3.12
```

Verify:

```powershell
python --version
```

### 2. Install the translation packages

```powershell
python -m pip install ctranslate2 transformers sentencepiece huggingface_hub
```

Roughly 200 MB of disk.

### 3. First model download

Happens automatically the first time you toggle **Translate** on (or click **Init translator**):

- Downloads `JustFrederik/nllb-200-distilled-600M-ct2-int8` from Hugging Face to `%USERPROFILE%\.cache\anuvaad\nllb-200-distilled-600M-ct2\`.
- ~600 MB. Subsequent runs use the cached copy.

## GPU vs CPU

CTranslate2 attempts CUDA automatically and falls back to CPU on failure. Expected per-sentence latency on a ~10-word line:

| Hardware | Latency |
|---|---|
| RTX 40-series / 50-series, int8_float16 | ~30 ms |
| CPU (8 cores), int8 | ~150–400 ms |

The translator status line (small, under the controls) reports which device it landed on.

## Supported languages

The UI exposes a curated subset of NLLB-200 (~30 popular languages). NLLB itself supports ~200; to add more, edit `LANGUAGES` in [`src/translator.rs`](https://github.com/tonybenoy/anuvaad/blob/main/src/translator.rs). The full FLORES-200 code list is [here](https://github.com/facebookresearch/flores/blob/main/flores200/README.md).

## Troubleshooting

**"translator.py not found"** — Make sure `scripts/translator.py` lives next to `anuvaad.exe`, or run the binary from the repo root.

**"missing python dep"** — Run the `pip install` line above. The status bar shows the exact missing module.

**Quality issues on short phrases** — NLLB-200-distilled-600M is the small variant. For better quality, swap to the 1.3B or 3.3B model — edit `MODEL_REPO` in `scripts/translator.py`.
