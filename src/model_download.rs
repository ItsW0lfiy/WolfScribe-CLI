use anyhow::{Context, Result, bail};
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

// Pin a known ONNX conversion rather than silently changing the model on users.
const MODEL_REVISION: &str = "8f23f0c03c8761650bdb5b40aaf3e40d2c15f1ce";
const MODEL_REPOSITORY: &str = "istupakov/parakeet-tdt-0.6b-v3-onnx";

// Conservative lower bounds reject empty files and common HTTP error bodies.
// Successful transfers are also checked by curl against HTTP Content-Length.
const REQUIRED_FILES: [(&str, u64); 4] = [
    ("encoder-model.onnx", 30_000_000),
    ("encoder-model.onnx.data", 2_000_000_000),
    ("decoder_joint-model.onnx", 50_000_000),
    ("vocab.txt", 70_000),
];

fn looks_complete(path: &Path, min_bytes: u64) -> bool {
    fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.len() >= min_bytes)
}

pub fn ensure_model_available(dir: &Path, allow_download: bool) -> Result<()> {
    let missing: Vec<(&str, u64)> = REQUIRED_FILES
        .iter()
        .copied()
        .filter(|(name, min_bytes)| !looks_complete(&dir.join(name), *min_bytes))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    if !allow_download {
        let names = missing.iter().map(|(name, _)| *name).collect::<Vec<_>>().join(", ");
        bail!(
            "Required model files are missing or incomplete in {}: {names}. Remove --offline, or install the model manually. --model-dir never auto-downloads.",
            dir.display()
        );
    }

    fs::create_dir_all(dir)
        .with_context(|| format!("Creating model directory {}", dir.display()))?;
    eprintln!(
        "WolfScribe needs Parakeet TDT 0.6B v3 (approximately 2.6 GB).\n\
         Download source: https://huggingface.co/{MODEL_REPOSITORY}\n\
         Model license: CC BY 4.0; see THIRD_PARTY_NOTICES.md.\n\
         Destination: {}\n\
         Downloading now; press Ctrl+C to cancel (partial downloads will be resumed).",
        dir.display()
    );

    for (name, min_bytes) in missing {
        let final_path = dir.join(name);
        let partial_path = dir.join(format!("{name}.part"));
        let url = format!(
            "https://huggingface.co/{MODEL_REPOSITORY}/resolve/{MODEL_REVISION}/{name}"
        );
        eprintln!("\nDownloading {name}...");
        let curl = if cfg!(windows) { "curl.exe" } else { "curl" };
        let status = Command::new(curl)
            .args([
                "--fail",
                "--location",
                "--retry",
                "3",
                "--retry-all-errors",
                "--retry-delay",
                "2",
                "--connect-timeout",
                "30",
                "--continue-at",
                "-",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
            ])
            .arg("--output")
            .arg(&partial_path)
            .arg(&url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .status()
            .with_context(|| {
                format!("Could not launch {curl}. Install curl, or provide --model-dir manually")
            })?;
        if !status.success() {
            bail!(
                "Download of {name} failed with {status}. A partial download was kept at {} for retry.",
                partial_path.display()
            );
        }
        if !looks_complete(&partial_path, min_bytes) {
            bail!(
                "Downloaded {name} is unexpectedly small. Keeping {} for inspection; model not installed.",
                partial_path.display()
            );
        }
        if final_path.exists() {
            fs::remove_file(&final_path)
                .with_context(|| format!("Replacing incomplete {}", final_path.display()))?;
        }
        fs::rename(&partial_path, &final_path)
            .with_context(|| format!("Installing model file {}", final_path.display()))?;
    }

    eprintln!("Model installed successfully. Future runs will use the local copy.\n");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{REQUIRED_FILES, looks_complete};
    use std::path::Path;

    #[test]
    fn missing_files_are_not_marked_complete() {
        assert!(!looks_complete(Path::new("/nonexistent/wolfscribe-test-file"), 1));
    }

    #[test]
    fn model_manifest_has_required_files() {
        assert_eq!(REQUIRED_FILES.len(), 4);
        assert!(REQUIRED_FILES.iter().all(|(_, length)| *length > 0));
    }
}
