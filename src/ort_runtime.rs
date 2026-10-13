//! ONNX Runtime 1.30.0 CUDA 13 (SM 120a/Blackwell) Windows bootstrap.
//! Uses a pinned Microsoft release ZIP and an app-private versioned cache.
//! GPU-equipped CUDA builds load the verified newer runtime with ort::init_from;
//! no installer, system PATH, or package-manager changes are needed.

use anyhow::{Context, Result, anyhow, bail};
use crate::backend::Device;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const VERSION: &str = "onnxruntime-1.30.0-cuda13-win-x64";
const ARCHIVE: &str = "onnxruntime-win-x64-gpu_cuda13-1.30.0.zip";
const URL: &str = "https://github.com/microsoft/onnxruntime/releases/download/v1.30.0/onnxruntime-win-x64-gpu_cuda13-1.30.0.zip";
const SIZE: u64 = 294_386_823;
const SHA256: &str = "8fa4b08359af682cd605892cb59077049700b640128bb93fd2c7776cf9f55bdc";
const DLLS: [&str; 3] = [
    "onnxruntime.dll",
    "onnxruntime_providers_cuda.dll",
    "onnxruntime_providers_shared.dll",
];

fn local_root() -> Result<PathBuf> {
    Ok(PathBuf::from(env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is not defined")?)
        .join("WolfScribe"))
}

fn verified_install(dir: &Path) -> bool {
    dir.join("installed.ok").is_file()
        && DLLS.iter().all(|name| dir.join(name).is_file())
}

fn hash_output(s: &str) -> Option<String> {
    s.lines().map(str::trim)
        .map(|line| line.chars().filter(|c| !c.is_ascii_whitespace()).collect::<String>())
        .find(|line| line.len() == 64 && line.chars().all(|c| c.is_ascii_hexdigit()))
        .map(|line| line.to_ascii_lowercase())
}

fn verified_archive(path: &Path) -> Result<bool> {
    if !fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() == SIZE) {
        return Ok(false);
    }
    let result = Command::new("certutil.exe")
        .arg("-hashfile").arg(path).arg("SHA256")
        .stdin(Stdio::null()).output()
        .context("Unable to run Windows certutil.exe")?;
    if !result.status.success() {
        bail!("Could not SHA-256 verify {}", path.display());
    }
    Ok(hash_output(&String::from_utf8_lossy(&result.stdout)).as_deref() == Some(SHA256))
}

fn download(cache: &Path) -> Result<PathBuf> {
    fs::create_dir_all(cache)?;
    // Reuse the ZIP downloaded by the earlier isolated Blackwell test.
    let archive = cache.join(ARCHIVE);
    if verified_archive(&archive)? {
        return Ok(archive);
    }
    if archive.exists() { fs::remove_file(&archive)?; }

    let part = cache.join(format!("{ARCHIVE}.part"));
    if fs::metadata(&part).is_ok_and(|m| m.len() >= SIZE) {
        fs::remove_file(&part)?;
    }
    eprintln!("Installing Blackwell-compatible ONNX Runtime 1.30.0 (~294 MB compressed)...");
    eprintln!("Official Microsoft source: {URL}");
    let result = Command::new("curl.exe")
        .args(["--fail", "--location", "--retry", "3", "--retry-all-errors",
               "--connect-timeout", "30", "--continue-at", "-", "--proto",
               "=https", "--proto-redir", "=https", "--output"])
        .arg(&part).arg(URL).stdin(Stdio::null()).status()
        .context("Unable to run Windows curl.exe")?;
    if !result.success() {
        bail!("ONNX Runtime download failed ({result}); resumable archive kept at {}", part.display());
    }
    if !verified_archive(&part)? {
        fs::remove_file(&part).ok();
        bail!("Official ONNX Runtime archive failed size/SHA-256 verification");
    }
    fs::rename(&part, &archive).context("Move verified ONNX Runtime archive into cache")?;
    Ok(archive)
}

fn find_dll_set(root: &Path) -> Result<PathBuf> {
    let mut queue = vec![root.to_owned()];
    while let Some(folder) = queue.pop() {
        if DLLS.iter().all(|dll| folder.join(dll).is_file()) {
            return Ok(folder);
        }
        for entry in fs::read_dir(&folder)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                queue.push(entry.path());
            }
        }
    }
    bail!("Official ONNX Runtime ZIP did not contain all three matching DLLs")
}

fn install(root: &Path) -> Result<PathBuf> {
    let destination = root.join("runtimes").join(VERSION);
    if verified_install(&destination) {
        return Ok(destination);
    }
    let archive = download(&root.join("downloads"))?;
    let temp = root.join("runtimes").join(format!("{VERSION}-staging"));
    if temp.exists() { fs::remove_dir_all(&temp)?; }
    fs::create_dir_all(&temp)?;

    let result = Command::new("tar.exe")
        .arg("-xf").arg(&archive).arg("-C").arg(&temp)
        .stdin(Stdio::null()).status()
        .context("Unable to run Windows tar.exe")?;
    if !result.success() {
        bail!("Extraction of SHA-256-verified ONNX Runtime archive failed");
    }
    let dll_path = find_dll_set(&temp)?;
    let staged = root.join("runtimes").join(format!("{VERSION}-ready"));
    if staged.exists() { fs::remove_dir_all(&staged)?; }
    fs::create_dir_all(&staged)?;
    for name in DLLS {
        fs::copy(dll_path.join(name), staged.join(name))
            .with_context(|| format!("Copy verified ONNX Runtime DLL {name}"))?;
    }
    fs::write(staged.join("installed.ok"), b"Microsoft ONNX Runtime 1.30.0 CUDA13; official SHA256-verified ZIP\n")?;
    if destination.exists() { fs::remove_dir_all(&destination)?; }
    fs::rename(&staged, &destination)?;
    fs::remove_dir_all(&temp)?;
    eprintln!("Blackwell-compatible ONNX Runtime cached in {}", destination.display());
    Ok(destination)
}

/// Load ONNX Runtime before any model/session is created.
/// A 1.30.0 DLL inside the application distribution is preferred. Development
/// builds still get a fallback to the existing older DLL for CPU operation
/// when --offline is set or automatic setup fails.
pub(crate) fn initialize(requested: Device, offline: bool) -> Result<()> {
    let base = env::current_exe()
        .context("Find WolfScribe executable")?
        .parent().context("Find WolfScribe executable directory")?.to_path_buf();
    let local = local_root()?;
    let packaged = base.join("runtime").join("onnxruntime.dll");
    let cached = local.join("runtimes").join(VERSION);
    let wants_gpu = requested != Device::Cpu && crate::cuda_runtime::has_nvidia_gpu();

    let selected = if packaged.is_file() {
        // For eventual portable ZIP / WinGet packages with ORT 1.30 beside app.
        packaged
    } else if verified_install(&cached) {
        cached.join("onnxruntime.dll")
    } else if wants_gpu && !offline {
        match install(&local) {
            Ok(path) => path.join("onnxruntime.dll"),
            Err(err) if requested == Device::Auto => {
                eprintln!("ONNX Runtime 1.30 automatic setup unavailable ({err}). Trying existing runtime/CPU fallback.");
                base.join("onnxruntime.dll")
            }
            Err(err) => return Err(err.context("CUDA mode requires a Blackwell-compatible ONNX Runtime")),
        }
    } else {
        base.join("onnxruntime.dll")
    };

    if !selected.is_file() {
        bail!(
            "ONNX Runtime DLL not found at {}. Build with CUDA enabled or run online once to install the verified runtime.",
            selected.display()
        );
    }

    // Providers and external CUDA libraries must be discoverable before
    // ONNX Runtime registers its CUDA execution provider.
    if let Some(parent) = selected.parent() {
        crate::cuda_runtime::add_dll_directory(parent)?;
    }
    if let Err(e) = crate::cuda_runtime::activate_cached() {
        eprintln!("NVIDIA CUDA runtime cache unavailable: {e}");
    }

    eprintln!("ONNX Runtime: {}", selected.display());
    ort::init_from(&selected)
        .map_err(|err| anyhow!("Failed loading ONNX Runtime {}: {err}", selected.display()))?
        .commit()
        .context("Could not initialize the selected ONNX Runtime DLL")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{DLLS, SHA256, SIZE, hash_output};
    #[test]
    fn pinned_official_runtime_has_expected_manifest() {
        assert_eq!(DLLS.len(), 3);
        assert_eq!(SHA256.len(), 64);
        assert!(SHA256.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(SIZE, 294_386_823);
    }
    #[test]
    fn parse_windows_certutil_sha256() {
        assert_eq!(
            hash_output(&format!("SHA256 hash of file\n{SHA256}\nCertUtil: success")).as_deref(),
            Some(SHA256)
        );
        assert!(hash_output("no hash").is_none());
    }
}
