# WolfScribe CLI

Rust-based, local audio/video transcription using NVIDIA Parakeet TDT 0.6B v3.

**Status: experimental v0.1 prototype.** CPU long-form transcription works, but subtitle timing and boundary deduplication still need verification. CUDA and crash recovery are not implemented. First-run automatic ONNX model downloading is available but has not yet been tested end-to-end with a fresh installation.

## Requirements

- Rust toolchain
- FFmpeg on PATH
- Internet on first run for the automatic model download (approximately 2.6 GB), or a preinstalled ONNX model
- `curl` on PATH for the first-run download (included in current Windows 11 installations)

FFmpeg handles decoding for audio and video formats such as WebM, MP4, WAV, MP3, and MKV. The CLI uses in-memory chunks rather than creating temporary WAV files.

## Build

    cargo build --release

After building, the Windows executable is target/release/wolfscribe.exe.

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

## Attribution and licensing

The Parakeet model is licensed separately under Creative Commons Attribution 4.0 International. See THIRD_PARTY_NOTICES.md.

WolfScribe CLI is independent of NVIDIA and is not endorsed or officially supported by NVIDIA.

**The license for the WolfScribe source code is not yet selected.**
