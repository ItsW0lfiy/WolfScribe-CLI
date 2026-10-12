# WolfScribe CLI

Rust-based, local audio/video transcription using NVIDIA Parakeet TDT 0.6B v3.

**Status: experimental v0.1 prototype.** CPU long-form transcription works, but subtitle timing and boundary deduplication still need verification. CUDA, auto-downloading models, and crash recovery are not implemented.

## Requirements

- Rust toolchain
- FFmpeg on PATH
- NVIDIA Parakeet TDT 0.6B v3 ONNX model files (not bundled with this repository)

FFmpeg handles decoding for audio and video formats such as WebM, MP4, WAV, MP3, and MKV. The CLI uses in-memory chunks rather than creating temporary WAV files.

## Build

    cargo build --release

After building, the Windows executable is target/release/wolfscribe.exe.

## Set up the model (Windows PowerShell)

Use the already-downloaded model as a temporary source, or download the appropriate ONNX model from https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx.

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

Optionally set WOLFSCRIBE_MODEL_DIR to override the default model path. Advanced controls: --chunk-seconds and --context-seconds.

The prototype writes SRT and TXT beside the input. **Existing outputs are overwritten.** Back up valuable transcripts before rerunning.

## Attribution and licensing

The Parakeet model is licensed separately under Creative Commons Attribution 4.0 International. See THIRD_PARTY_NOTICES.md.

WolfScribe CLI is independent of NVIDIA and is not endorsed or officially supported by NVIDIA.

**The license for the WolfScribe source code is not yet selected.**
