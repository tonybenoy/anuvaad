# Build from source

anuvaad is a Rust + cpal + whisper.cpp project. On Windows the dependency chain is real; this page walks through it.

## Prerequisites

All installable via `winget`. Open an Administrator PowerShell:

```powershell
# Rust toolchain
winget install Rustlang.Rustup
rustup default stable

# MSVC C/C++ compiler — required by Rust on Windows
winget install Microsoft.VisualStudio.BuildTools -e --override `
  "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"

# LLVM — provides libclang.dll for bindgen (whisper-rs)
winget install LLVM.LLVM

# CMake — used by whisper-rs-sys to build whisper.cpp
winget install Kitware.CMake

# CUDA Toolkit — only if you want --features cuda
winget install Nvidia.CUDA
```

After installing, open a **fresh** PowerShell so the new PATH entries are visible.

## Set environment variables

`bindgen` needs to find `libclang.dll`:

```powershell
$env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin"
```

For a permanent setting:

```powershell
[Environment]::SetEnvironmentVariable("LIBCLANG_PATH", "C:\Program Files\LLVM\bin", "User")
```

If you're building with CUDA, also confirm `CUDA_PATH` exists (the toolkit installer sets it):

```powershell
echo $env:CUDA_PATH
# C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.2
```

## Build

```powershell
git clone https://github.com/tonybenoy/anuvaad
cd anuvaad

# CPU build — works anywhere
cargo build --release

# CUDA build — needs CUDA Toolkit installed
cargo build --release --features cuda
```

First build takes ~5–10 minutes (whisper.cpp compiles fresh). Subsequent builds are ~10 seconds for code-only changes.

## Output

```
target/release/anuvaad.exe       (~10 MB CPU / ~38 MB CUDA)
```

CUDA kernels and whisper.cpp are statically linked. No runtime DLL dependencies beyond what comes with Windows + the NVIDIA driver.

## Build the docs site

```powershell
cargo install mdbook
mdbook serve --open
```

mdBook watches `docs/` and rebuilds on save.

## Cross-platform note

cpal and whisper.cpp work on macOS and Linux, but anuvaad currently uses WASAPI-specific loopback for system audio capture. Adapting to PulseAudio/PipeWire (Linux) or BlackHole/CoreAudio (macOS) is straightforward but not done yet.
