# Contributing

This is a single-developer hobby project. PRs welcome but please open an issue first if you're planning more than a one-line fix.

## Development setup

See [Build from source](build-from-source.md).

For active development:

```powershell
cargo run --release                  # CPU
cargo run --release --features cuda  # GPU
```

The release profile is heavily preferred even for dev because:
- whisper.cpp is essentially unusable in debug mode (10× slower)
- egui rendering is much smoother
- LTO is on, so iteration is still fast for small Rust changes

## Code style

- `cargo fmt` before committing.
- `cargo clippy --features cuda -- -D warnings` should pass.

## Areas where help would be especially welcome

- macOS support (BlackHole or CoreAudio aggregate device for loopback)
- Linux support (PulseAudio monitor or PipeWire)
- Word-level streaming (currently segment-level)
- Tray icon / global hotkeys for start/stop
- Better silence detection (proper VAD model instead of RMS gate)
- Tests (currently zero — embarrassing)

## Filing bugs

Include:
- OS version + GPU model (if relevant)
- Output of `nvidia-smi` if CUDA-related
- Output of `python -c "import ctranslate2; print(ctranslate2.__version__)"` if translation-related
- Console output (`cargo run --release 2>&1 | tee log.txt`) — anuvaad logs errors to stderr
