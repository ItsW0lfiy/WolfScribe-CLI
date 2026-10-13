# Third-party notices

## NVIDIA Parakeet TDT 0.6B v3

Model creator: NVIDIA
Original model: https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3
License: Creative Commons Attribution 4.0 International (CC BY 4.0)
License text: https://creativecommons.org/licenses/by/4.0/legalcode.en
ONNX conversion used in testing: https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx

Model weights are not included in this repository. Redistribution of model weights or conversions must preserve required attribution, notices, links, and modification information where applicable.

WolfScribe CLI is an independent project. NVIDIA does not sponsor, endorse, or officially support it.

The model's license does not determine the license of the Rust application.

## Microsoft ONNX Runtime 1.30.0 (Windows portable ZIP)

The official Windows release package bundles ONNX Runtime's matching
CUDA 13 DLLs from https://github.com/microsoft/onnxruntime/releases/tag/v1.30.0
(verified against Microsoft's published archive SHA-256).
License: MIT; the official upstream license is also included at
runtime/LICENSE-ONNXRUNTIME.txt.

## FFmpeg

FFmpeg is a separately installed dependency, **not** bundled with this
Windows ZIP. The suggested WinGet package is Gyan.FFmpeg.Essentials.
Its GPL licensing, corresponding source and build details are provided by
its independent distributor: https://ffmpeg.org/legal.html and
https://www.gyan.dev/ffmpeg/builds/.

## NVIDIA CUDA and cuDNN

No NVIDIA CUDA or cuDNN redistributables are bundled. WolfScribe can
download pinned NVIDIA components after legally eligible user acceptance
of the terms. The model is also downloaded separately.
