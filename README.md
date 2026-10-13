# WolfScribe CLI

Rust-based, local audio/video transcription using NVIDIA Parakeet TDT 0.6B v3.

**Status: v0.1 release candidate.** Windows RTX 5070 CUDA inference is verified with integrated ONNX Runtime 1.30.0, including a 33-minute WebM processed successfully in 16 seconds. CPU fallback is implemented; subtitle timing, boundary deduplication and first-release packaging need final verification.

## Requirements

- Rust toolchain
- FFmpeg next to the WolfScribe executable, in the development `tools` folder, or on PATH
- Internet on first run for the automatic model download (approximately 2.6 GB), or a preinstalled ONNX model
- `curl` on PATH for the first-run download (included in current Windows 11 installations)

FFmpeg handles decoding for audio and video formats such as WebM, MP4, WAV, MP3, and MKV. WolfScribe first looks beside its executable for a bundled `ffmpeg.exe`, then in the `tools/` directory of the current working directory (developer testing), and finally uses FFmpeg on PATH. Official distributions should package the appropriate licensed FFmpeg executable alongside WolfScribe; do not commit a compiled FFmpeg binary into this source repository. The CLI uses in-memory chunks rather than creating temporary WAV files.

## Build

    cargo build --release

For an NVIDIA GPU, build the CUDA-enabled variant:

    cargo build --release --features cuda

After building, the Windows executable is target/release/wolfscribe.exe. CPU support remains available in both builds.

## First run: automatic model download

Just run the executable with your media file. If the required model files are absent from the default per-user directory, WolfScribe downloads them from the pinned Hugging Face ONNX repository and displays progress. Interrupted transfers leave `.part` files for the next attempt. Subsequent runs do not need Internet access. Downloading uses `curl` and requires approximately 2.6 GB of disk space.

    .\target\release\wolfscribe.exe 'C:\media\recording.webm'

Use `--offline` to forbid network access; it will report a missing model instead. Supplying `--model-dir` also prevents auto-downloading into your custom directory.

**Model verification note:** this first version checks downloaded files for nontrivial sizes and uses curl's transfer checks, but it does not yet perform cryptographic SHA-256 verification. Do not treat this prototype as a security-hardened installer.

## Manual model setup (optional; Windows PowerShell)

**You can skip this section completely** if you're happy with first-run automatic installation. Alternatively, copy the model from a previous test/download directory or download it manually from https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx.

    $dest = Join-Path $env:LOCALAPPDATA 'WolfScribe\models\parakeet-tdt-v3'
    New-Item -ItemType Directory -Force $dest | Out-Null
    $source = 'C:\Projects\ParakeetBench\tdt'
    'encoder-model.onnx', 'encoder-model.onnx.data', 'decoder_joint-model.onnx', 'vocab.txt' |
        ForEach-Object { Copy-Item (Join-Path $source $_) $dest -Force }

**Do not delete ParakeetBench until the new location has been tested.** This copy is only needed once.

The model path defaults to %LOCALAPPDATA%/WolfScribe/models/parakeet-tdt-v3 on Windows. On Linux it defaults to $XDG_DATA_HOME/wolfscribe/models/parakeet-tdt-v3, or ~/.local/share/wolfscribe/models/parakeet-tdt-v3.

## Usage

    .\target\release\wolfscribe.exe 'C:\media\recording.webm'
    .\target\release\wolfscribe.exe 'C:\media\recording.mp4' --model-dir 'C:\models\parakeet'

Optionally set WOLFSCRIBE_MODEL_DIR to override the default model path. Advanced controls: --chunk-seconds, --context-seconds, and --offline. A custom model directory is never automatically populated.

The prototype writes SRT and TXT beside the input. **Existing outputs are overwritten.** Back up valuable transcripts before rerunning.

## Batch transcription

Process several files or one directory without loading Parakeet for each file:

    .\target\release\wolfscribe.exe 'C:\Recordings\one.wav' 'C:\Recordings\two.webm'
    .\target\release\wolfscribe.exe 'C:\Recordings' --device auto

Directories are non-recursive. Supported media extensions are discovered
automatically; direct file paths may use any format FFmpeg can decode.
Duplicate paths are ignored. Every input gets its own SRT/TXT beside the
media file. Existing output files are overwritten. A collision (such as
same.mp4 and same.webm producing the same SRT) is rejected before processing.
When one file fails, the rest are still processed, and the final exit status
reports failure.

## First public release (preparation)

The public v0.1.0 release is not published yet. The project owner must
select and approve a source LICENSE, and the distributable must pass final
smoke tests. The GitHub Actions **Build Windows Release** workflow creates
an unsigned Windows x64 portable ZIP on manual invocation. After license
approval, pushing a matching version tag (v0.1.0) also publishes a GitHub
release automatically.

The package contains WolfScribe, the official SHA-256-verified Microsoft
ONNX Runtime 1.30 CUDA 13 DLLs (including CPU fallback), notices and
licenses. It deliberately does not bundle NVIDIA's ~1.33-GB CUDA/cuDNN
components or Parakeet's ~2.6-GB model; those are provisioned as needed
with their separate licenses. FFmpeg is an external dependency instead of
being bundled under another project's GPL distribution obligations. Install:

    winget install --id Gyan.FFmpeg.Essentials -e

For WinGet publication, once the GitHub release ZIP is downloadable,
generate versioned manifests with the **actual ZIP hash**, e.g.:

    .\scripts\New-WingetManifest.ps1 -ZipPath .\target\dist\WolfScribe-CLI-0.1.0-windows-x64.zip -Version 0.1.0 -License 'APPROVED_SPDX_ID'
    winget validate --manifest .\target\winget-manifests\manifests\i\ItsW0lfiy\WolfScribe\0.1.0

Then test installation from the manifest and submit its three files in a
single PR to https://github.com/microsoft/winget-pkgs. Inclusion in the
public WinGet catalog is subject to independent Microsoft approval.
No placeholder asset URL or fabricated SHA-256 may be submitted.

## CPU and CUDA backends

The `--device` option selects inference mode:

    .\target\release\wolfscribe.exe 'C:\media\recording.webm' --device cpu
    .\target\release\wolfscribe.exe 'C:\media\recording.webm' --device cuda
    .\target\release\wolfscribe.exe 'C:\media\recording.webm' --device auto

- `cpu` always uses CPU.
- `cuda` requires a `--features cuda` build and requests **strict CUDA execution-provider registration**. If the provider cannot register, it exits with an error rather than silently benchmarking CPU.
- `auto` (default) attempts CUDA when compiled with CUDA support. If GPU initialization fails, it offers a pinned, verified first-run NVIDIA runtime download on supported Windows/NVIDIA systems, then falls back to CPU if setup is declined, unavailable or unsuccessful. If a CUDA kernel fails **during transcription**, `auto` now discards the GPU session, reloads the CPU backend, and retries the same audio chunk. Previous completed chunks and output remain intact. Explicit `--device cuda` does not silently fall back. In a normal CPU-only build, `auto` uses CPU.

**Important:** A successful CUDA provider registration does **not** guarantee that every model operator runs on the GPU. ONNX Runtime may assign unsupported operators to CPU. The CLI reports the configured backend rather than claiming full GPU utilization; verify usage with `nvidia-smi` and compare elapsed timings.

CUDA builds require an **ONNX Runtime GPU-capable library** and compatible CUDA/cuDNN runtime DLLs. On Windows x64, WolfScribe automatically downloads and verifies the official Microsoft ONNX Runtime 1.30.0 CUDA 13 release (~294 MB, one time), selecting it at startup with runtime dynamic loading. This is independent of the NVIDIA CUDA/cuDNN download and does not require modifying the system installation. On Windows x64, WolfScribe can download NVIDIA CUDA 13.0 component archives and cuDNN 9.17.1 from official NVIDIA distribution URLs with pinned archive sizes and SHA-256 hashes. The runtime is extracted to `%LOCALAPPDATA%\WolfScribe\runtimes\`, not system-wide. Downloads use Windows built-in `curl.exe`, `certutil.exe` and `tar.exe`. The first download is approximately 1.33 GB compressed, in addition to the speech model; provide enough free space for extraction.

**NVIDIA licenses:** the terminal displays links to CUDA and cuDNN legal terms and prompts for eligible acceptance before first downloading. NVIDIA's SDK license has age/legal-capacity requirements. For CI/noninteractive use, `--accept-nvidia-license` is available only to users legally entitled to accept both agreements. No acceptance means CPU fallback in `auto` mode; explicit `cuda` reports an actionable error. `--offline` never downloads. The app does not alter your driver or global PATH.

WolfScribe now dynamically loads **ONNX Runtime 1.30.0 CUDA 13** from a verified versioned per-user cache on Windows when compiled with `--features cuda` (or from a packaged `runtime/` directory if present). Its official Windows binary has successfully completed GPU transcription on an RTX 5070 Laptop. This same runtime is now used by the normal executable instead of requiring `target/blackwell-test`; the integrated path has passed physical RTX 5070 hardware tests. CUDA downloads are automatic on first run unless `--offline` is set. The mid-transcription CPU fallback has also been validated. See https://onnxruntime.ai/docs/execution-providers/CUDA-ExecutionProvider.html for NVIDIA/ONNX compatibility details.

CUDA builds pass CI and CUDA inference has been verified on an RTX 5070 Laptop. Benchmarks should use the same file, chunk/context settings, and system load, comparing `--device cpu` and `--device cuda`.

## Experimental RTX 50 / Blackwell CUDA trial

ONNX Runtime 1.30.0 official Windows CUDA 13 release notes list Blackwell SM 120a support. The current Rust binding targets ONNX Runtime 1.28, so test the newer official DLLs separately before making any release-wide change.

    cargo build --release --locked --features cuda
    .\scripts\Test-BlackwellCuda.ps1
    .\target\blackwell-test\wolfscribe.exe 'C:\media\speech.wav' --device cuda --offline

The developer script downloads the official Microsoft 1.30.0 CUDA 13 ZIP (approximately 294 MB), verifies its exact GitHub release size and SHA-256, and copies its three matching DLLs beside a copy of WolfScribe under target/blackwell-test. It does not overwrite target/release, touch system PATH, or change NVIDIA runtime caches. A previously installed/cached NVIDIA CUDA runtime is still needed.

Do not use auto for this test: strict cuda mode makes any remaining GPU error obvious. The normal CUDA-enabled WolfScribe executable now installs and loads ONNX Runtime 1.30 from the private cache automatically; the script above remains for isolated regression tests. This is not yet a packaged public release.

## Attribution and licensing

The Parakeet model is licensed separately under Creative Commons Attribution 4.0 International. See THIRD_PARTY_NOTICES.md.

WolfScribe CLI is independent of NVIDIA and is not endorsed or officially supported by NVIDIA.

**The license for the WolfScribe source code is not yet selected.**
