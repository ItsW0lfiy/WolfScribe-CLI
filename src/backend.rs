use anyhow::{Result, anyhow};
use clap::ValueEnum;
use parakeet_rs::ParakeetTDT;
#[cfg(feature = "cuda")]
use parakeet_rs::ExecutionConfig;
use std::path::Path;

/// Requested inference backend. Auto attempts CUDA only in CUDA-enabled builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Device {
    Auto,
    Cpu,
    Cuda,
}

pub(crate) struct LoadedModel {
    pub(crate) model: ParakeetTDT,
    /// Describes registration, not proof every ONNX operator ran on this device.
    pub(crate) backend: &'static str,
    pub(crate) uses_cuda: bool,
}

fn cpu(model_dir: &Path) -> Result<LoadedModel> {
    let model = ParakeetTDT::from_pretrained(model_dir, None)
        .map_err(|error| anyhow!("Failed loading Parakeet on CPU: {error}"))?;
    Ok(LoadedModel {
        model,
        backend: "CPU",
        uses_cuda: false,
    })
}

#[cfg(feature = "cuda")]
fn cuda(model_dir: &Path) -> Result<LoadedModel> {
    // parakeet-rs normally requests CUDA with silent CPU fallback on registration
    // failure. Use an explicit, strict EP registration instead, so we never
    // report CUDA when ONNX Runtime couldn't register the CUDA provider.
    // ONNX Runtime can still assign unsupported nodes to its CPU provider.
    let config = ExecutionConfig::new().with_custom_configure(|builder| {
        Ok(builder.with_execution_providers([
            ort::ep::CUDA::default().build().error_on_failure(),
            ort::ep::CPU::default().build().error_on_failure(),
        ])?)
    });
    let model = ParakeetTDT::from_pretrained(model_dir, Some(config))
        .map_err(|error| anyhow!("CUDA provider/model initialization failed: {error}"))?;
    Ok(LoadedModel {
        model,
        backend: "CUDA execution provider registered (CPU may handle unsupported operators)",
        uses_cuda: true,
    })
}

#[cfg(not(feature = "cuda"))]
fn cuda(_model_dir: &Path) -> Result<LoadedModel> {
    Err(anyhow!("This executable was built without CUDA support. Rebuild with: cargo build --release --features cuda"))
}

#[cfg(feature = "cuda")]
fn cuda_with_setup(model_dir: &Path, offline: bool, accepted: bool) -> Result<LoadedModel> {
    #[cfg(windows)]
    if let Err(error) = crate::cuda_runtime::activate_cached() {
        eprintln!("Cached CUDA runtime could not be activated: {error}");
    }
    match cuda(model_dir) {
        Ok(model) => Ok(model),
        Err(first_error) => {
            #[cfg(windows)]
            {
                let message = first_error.to_string();
                // Provision runtime libraries only for missing native DLLs,
                // not for OOM, incompatible GPU, or unrelated model errors.
                let missing_library = message.contains("which is missing")
                    || message.contains("Error 126")
                    || message.contains("The specified module could not be found");
                if missing_library && !offline {
                    let installed = crate::cuda_runtime::install(false, accepted)
                        .map_err(|setup| anyhow!("CUDA initialization failed: {first_error}. Automatic CUDA setup failed: {setup}"))?;
                    if installed {
                        return cuda(model_dir).map_err(|retry| {
                            anyhow!("CUDA libraries installed, but CUDA initialization still failed: {retry}")
                        });
                    }
                    return Err(anyhow!("CUDA libraries missing; NVIDIA license/download was not accepted. {first_error}"));
                }
            }
            Err(anyhow!("CUDA cannot initialize: {first_error}{}", if offline { " (--offline prevents automatic CUDA setup)" } else { "" }))
        }
    }
}

/// Runtime fallback is for automatically selected CUDA only. Explicit
/// --device cuda must remain strict and report inference failure.
pub(crate) fn should_fallback_on_inference_error(requested: Device, uses_cuda: bool) -> bool {
    requested == Device::Auto && uses_cuda
}

pub(crate) fn summarize_gpu_failure(error: &str) -> &'static str {
    if error.contains("cudaErrorNoKernelImageForDevice") {
        "A CUDA kernel is incompatible with this GPU architecture. This can happen when ONNX Runtime's prebuilt CUDA provider lacks support for your GPU (e.g. RTX 50 / Blackwell). Installing more CUDA DLLs won't fix a missing kernel image."
    } else {
        "CUDA inference failed. The automatically selected GPU backend will be retired for the remainder of this run."
    }
}

pub(crate) fn load_model(model_dir: &Path, requested: Device, offline: bool, accepted: bool) -> Result<LoadedModel> {
    #[cfg(not(feature = "cuda"))]
    let _ = (offline, accepted);

    match requested {
        Device::Cpu => cpu(model_dir),
        Device::Cuda => {
            #[cfg(feature = "cuda")]
            { cuda_with_setup(model_dir, offline, accepted) }
            #[cfg(not(feature = "cuda"))]
            { cuda(model_dir) }
        }
        Device::Auto => {
            #[cfg(feature = "cuda")]
            {
                match cuda_with_setup(model_dir, offline, accepted) {
                    Ok(loaded) => return Ok(loaded),
                    Err(error) => {
                        eprintln!("CUDA unavailable ({error}). Falling back to CPU.");
                    }
                }
            }
            #[cfg(not(feature = "cuda"))]
            eprintln!("CUDA is not compiled into this build; using CPU. Build with --features cuda to enable it.");
            cpu(model_dir)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Device, should_fallback_on_inference_error, summarize_gpu_failure};
    use clap::ValueEnum;

    #[test]
    fn retries_only_automatically_selected_cuda() {
        assert!(should_fallback_on_inference_error(Device::Auto, true));
        assert!(!should_fallback_on_inference_error(Device::Auto, false));
        assert!(!should_fallback_on_inference_error(Device::Cuda, true));
        assert!(!should_fallback_on_inference_error(Device::Cpu, false));
    }

    #[test]
    fn gives_actionable_missing_kernel_diagnostic() {
        let description = summarize_gpu_failure("cudaErrorNoKernelImageForDevice: no kernel image");
        assert!(description.contains("GPU architecture"));
    }

    #[test]
    fn device_options_are_stable() {
        for (option, expected) in [
            ("auto", Device::Auto),
            ("cpu", Device::Cpu),
            ("cuda", Device::Cuda),
        ] {
            assert_eq!(Device::from_str(option, false).unwrap(), expected);
        }
        assert!(Device::from_str("unknown", false).is_err());
    }

    #[cfg(not(feature = "cuda"))]
    #[test]
    fn explicit_cuda_requires_cuda_build() {
        let error = super::cuda(std::path::Path::new("."))
            .err()
            .expect("CPU-only build must reject CUDA");
        assert!(error.to_string().contains("--features cuda"));
    }
}
