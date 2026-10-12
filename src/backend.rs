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
}

fn cpu(model_dir: &Path) -> Result<LoadedModel> {
    let model = ParakeetTDT::from_pretrained(model_dir, None)
        .map_err(|error| anyhow!("Failed loading Parakeet on CPU: {error}"))?;
    Ok(LoadedModel {
        model,
        backend: "CPU",
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
    })
}

#[cfg(not(feature = "cuda"))]
fn cuda(_model_dir: &Path) -> Result<LoadedModel> {
    Err(anyhow!("This executable was built without CUDA support. Rebuild with: cargo build --release --features cuda"))
}

pub(crate) fn load_model(model_dir: &Path, requested: Device) -> Result<LoadedModel> {
    match requested {
        Device::Cpu => cpu(model_dir),
        Device::Cuda => cuda(model_dir),
        Device::Auto => {
            #[cfg(feature = "cuda")]
            {
                match cuda(model_dir) {
                    Ok(loaded) => return Ok(loaded),
                    Err(error) => {
                        eprintln!(
                            "CUDA unavailable ({error}). Falling back to CPU. \
                             Use --device cuda to make CUDA failure fatal."
                        );
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
    use super::Device;
    use clap::ValueEnum;

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
