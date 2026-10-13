//! Optional, privately cached NVIDIA CUDA/cuDNN runtime for Windows x64.
//! The archives, byte counts and SHA-256 digests are pinned from NVIDIA's
//! official CUDA 13.0.0 and cuDNN 9.17.1 redistrib JSON manifests.
//! This code must never download arbitrary DLL names from exception text.

use anyhow::{Context, Result, bail};
use std::env;
use std::ffi::c_void;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const ROOT: &str = "https://developer.download.nvidia.com/compute/";
const VERSION: &str = "cuda13.0-cudnn9.17.1-v1";
const LICENSE_CUDA: &str = "https://docs.nvidia.com/cuda/eula/index.html";
const LICENSE_CUDNN: &str = "https://docs.nvidia.com/deeplearning/cudnn/latest/reference/eula.html";

struct Archive {
    id: &'static str,
    path: &'static str,
    digest: &'static str,
    size: u64,
}

const ARCHIVES: [Archive; 6] = [
    Archive {
        id: "cuda_cudart",
        path: "cuda/redist/cuda_cudart/windows-x86_64/cuda_cudart-windows-x86_64-13.0.48-archive.zip",
        digest: "82fb29001895810ce02e88b180cbaa90607b852a7f67a63f18f202079eaa2966",
        size: 2_873_274,
    },
    Archive {
        id: "libcublas",
        path: "cuda/redist/libcublas/windows-x86_64/libcublas-windows-x86_64-13.0.0.19-archive.zip",
        digest: "c8fb12715b9639f51983315cc4b195e272128c675aa3766b13c4f470b892e6c8",
        size: 399_809_469,
    },
    Archive {
        id: "libcufft",
        path: "cuda/redist/libcufft/windows-x86_64/libcufft-windows-x86_64-12.0.0.15-archive.zip",
        digest: "4f0e0c019e1b53166bdeaf52c91c333450ae9dd05fbadc77cffb70a702501b67",
        size: 212_026_240,
    },
    Archive {
        id: "libcurand",
        path: "cuda/redist/libcurand/windows-x86_64/libcurand-windows-x86_64-10.4.0.35-archive.zip",
        digest: "c4db1ed0595fcdba3c4da0f5605ca4c3f4340b6be833d94765994e239c775a32",
        size: 54_762_976,
    },
    Archive {
        id: "cuda_nvrtc",
        path: "cuda/redist/cuda_nvrtc/windows-x86_64/cuda_nvrtc-windows-x86_64-13.0.48-archive.zip",
        digest: "d1ae61887c0d857f8c0cdb77247ef27fb7c78cc81ecb1a2786e81b7a9bea89a0",
        size: 318_082_272,
    },
    Archive {
        id: "cudnn",
        path: "cudnn/redist/cudnn/windows-x86_64/cudnn-windows-x86_64-9.17.1.4_cuda13-archive.zip",
        digest: "a1dbda7fb5672e236c40995d74f659daffdeea9432fcfe39731731daf12a8819",
        size: 342_411_920,
    },
];

const EXPECTED: [&str; 5] = [
    "cublasLt64_13.dll",
    "cublas64_13.dll",
    "cudart64_13.dll",
    "cufft64_12.dll",
    "cudnn64_9.dll",
];

#[link(name = "kernel32")]
unsafe extern "system" {
    fn SetDefaultDllDirectories(flags: u32) -> i32;
    fn AddDllDirectory(path: *const u16) -> *mut c_void;
}

fn cache_root() -> Result<PathBuf> {
    Ok(PathBuf::from(env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is not defined")?)
        .join("WolfScribe")
        .join("runtimes")
        .join(VERSION))
}

fn complete(root: &Path) -> bool {
    root.join("installed.ok").is_file()
        && EXPECTED.iter().all(|dll| root.join("bin").join(dll).is_file())
}

/// Makes a cached CUDA runtime visible to ONNX Runtime's Win32 DLL loader.
/// No global PATH modifications or administrator rights are needed.
pub(crate) fn activate_cached() -> Result<bool> {
    let root = cache_root()?;
    if !complete(&root) {
        return Ok(false);
    }
    let canonical = root.join("bin").canonicalize().context("Resolve cached CUDA bin directory")?;
    add_dll_directory(&canonical)?;
    Ok(true)
}

/// Add one explicit trusted DLL directory to the process search path.
/// The Win32 loader retains the cookie for the lifetime of this process.
pub(crate) fn add_dll_directory(directory: &Path) -> Result<()> {
    let canonical = directory.canonicalize().context("Resolve DLL directory")?;
    let mut path: Vec<u16> = canonical.as_os_str().encode_wide().collect();
    path.push(0);
    // LOAD_LIBRARY_SEARCH_DEFAULT_DIRS: app dir, System32 and user-added dirs.
    // These options avoid relying on the current working directory for DLLs.
    let success = unsafe { SetDefaultDllDirectories(0x0000_1000) };
    if success == 0 {
        bail!("SetDefaultDllDirectories failed: {}", io::Error::last_os_error());
    }
    let cookie = unsafe { AddDllDirectory(path.as_ptr()) };
    if cookie.is_null() {
        bail!("AddDllDirectory failed: {}", io::Error::last_os_error());
    }
    Ok(())
}

pub(crate) fn has_nvidia_gpu() -> bool {
    Command::new("nvidia-smi.exe")
        .arg("-L")
        .stdin(Stdio::null())
        .output()
        .is_ok_and(|out| out.status.success() && String::from_utf8_lossy(&out.stdout).contains("GPU "))
}

fn approval(already_approved: bool) -> Result<bool> {
    if already_approved {
        return Ok(true);
    }
    eprintln!(
        "WolfScribe can set up CUDA acceleration without system-wide installation.\n\
         NVIDIA runtime download: approximately 1.33 GB, plus extracted files.\n\
         NVIDIA CUDA license: {LICENSE_CUDA}\n\
         NVIDIA cuDNN license: {LICENSE_CUDNN}\n\
         These agreements may only be accepted by someone legally eligible to do so.\n\
         No changes are made to system PATH or the NVIDIA driver."
    );
    if !io::stdin().is_terminal() {
        eprintln!("Non-interactive session: no license acceptance. Skipping CUDA download.");
        return Ok(false);
    }
    eprint!("If you are legally eligible and accept BOTH license terms, type YES to download: ");
    io::stderr().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(line.trim() == "YES")
}

fn hash_output(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .map(|line| line.chars().filter(|c| !c.is_ascii_whitespace()).collect::<String>())
        .find(|line| line.len() == 64 && line.chars().all(|c| c.is_ascii_hexdigit()))
        .map(|line| line.to_ascii_lowercase())
}

fn verified(path: &Path, spec: &Archive) -> Result<bool> {
    if !fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.len() == spec.size) {
        return Ok(false);
    }
    let out = Command::new("certutil.exe")
        .arg("-hashfile")
        .arg(path)
        .arg("SHA256")
        .stdin(Stdio::null())
        .output()
        .context("Windows certutil is needed to verify NVIDIA archives")?;
    if !out.status.success() {
        bail!("SHA-256 verification command failed for {}", path.display());
    }
    Ok(hash_output(&String::from_utf8_lossy(&out.stdout)).is_some_and(|digest| digest == spec.digest))
}

fn download_verified(spec: &Archive, cache: &Path) -> Result<PathBuf> {
    fs::create_dir_all(cache)?;
    let destination = cache.join(format!("{}.zip.part", spec.id));
    if verified(&destination, spec)? {
        return Ok(destination);
    }
    // Keep incomplete downloads so curl can resume, but discard a complete
    // incorrect archive instead of trusting/using it.
    if fs::metadata(&destination).is_ok_and(|meta| meta.len() >= spec.size) {
        fs::remove_file(&destination)?;
    }
    let url = format!("{ROOT}{}", spec.path);
    eprintln!("Downloading NVIDIA {} from {url}", spec.id);
    let status = Command::new("curl.exe")
        .args([
            "--fail", "--location", "--retry", "3", "--retry-all-errors",
            "--connect-timeout", "30", "--continue-at", "-", "--proto",
            "=https", "--proto-redir", "=https", "--output",
        ])
        .arg(&destination)
        .arg(&url)
        .stdin(Stdio::null())
        .status()
        .context("Could not run curl.exe for NVIDIA runtime download")?;
    if !status.success() {
        bail!("NVIDIA download failed for {} ({status}); a partial archive was retained", spec.id);
    }
    if !verified(&destination, spec)? {
        fs::remove_file(&destination).ok();
        bail!("NVIDIA archive {} failed size/SHA-256 verification; refusing to extract", spec.id);
    }
    Ok(destination)
}

fn collect_binaries(directory: &Path, bin: &Path, licenses: &Path, id: &str) -> Result<usize> {
    let mut count = 0usize;
    for item in fs::read_dir(directory)? {
        let item = item?;
        let kind = item.file_type()?;
        if kind.is_symlink() {
            bail!("NVIDIA archive contained a link; refusing to follow it");
        }
        let path = item.path();
        if kind.is_dir() {
            count += collect_binaries(&path, bin, licenses, id)?;
        } else if kind.is_file() {
            let filename = item.file_name();
            let filename_str = filename.to_string_lossy();
            if path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("dll"))
                && path.parent().is_some_and(|parent| parent.components().any(|c| c.as_os_str().to_string_lossy().eq_ignore_ascii_case("bin")))
            {
                let target = bin.join(&filename);
                fs::copy(&path, &target).with_context(|| format!("Installing {}", target.display()))?;
                count += 1;
            } else if filename_str.eq_ignore_ascii_case("LICENSE.txt") {
                let target = licenses.join(format!("{id}-LICENSE.txt"));
                fs::copy(&path, target)?;
            }
        }
    }
    Ok(count)
}

fn extract_verified(archive: &Path, spec: &Archive, root: &Path) -> Result<()> {
    let temp = root.join(format!("extract-{}", spec.id));
    if temp.exists() {
        fs::remove_dir_all(&temp)?;
    }
    fs::create_dir_all(&temp)?;
    let status = Command::new("tar.exe")
        .arg("-xf")
        .arg(archive)
        .arg("-C")
        .arg(&temp)
        .stdin(Stdio::null())
        .status()
        .context("Windows tar.exe is required for ZIP extraction")?;
    if !status.success() {
        bail!("Could not extract verified NVIDIA archive {}", spec.id);
    }
    let bin = root.join("bin");
    let licenses = root.join("licenses");
    fs::create_dir_all(&bin)?;
    fs::create_dir_all(&licenses)?;
    let copied = collect_binaries(&temp, &bin, &licenses, spec.id)?;
    if copied == 0 {
        bail!("Verified NVIDIA archive {} contained no bin DLLs", spec.id);
    }
    fs::remove_dir_all(&temp)?;
    Ok(())
}

/// Called only after an actual CUDA initialization error, on systems with
/// NVIDIA hardware, and never for --offline mode.
pub(crate) fn install(offline: bool, accepted: bool) -> Result<bool> {
    if offline {
        bail!("--offline prevents downloading CUDA runtime components");
    }
    if !has_nvidia_gpu() {
        bail!("No NVIDIA GPU detected by nvidia-smi; skipping CUDA runtime download");
    }
    let root = cache_root()?;
    if complete(&root) {
        return activate_cached();
    }
    if !approval(accepted)? {
        return Ok(false);
    }

    fs::create_dir_all(&root)?;
    let archive_cache = root.join("archives");
    for spec in &ARCHIVES {
        let archive = download_verified(spec, &archive_cache)?;
        extract_verified(&archive, spec, &root)?;
        fs::remove_file(&archive)?;
    }
    let missing: Vec<_> = EXPECTED.iter()
        .filter(|name| !root.join("bin").join(name).is_file())
        .collect();
    if !missing.is_empty() {
        bail!("NVIDIA runtime setup incomplete; missing DLLs: {}", missing.into_iter().copied().collect::<Vec<_>>().join(", "));
    }
    fs::write(root.join("installed.ok"), b"cuda13.0-cudnn9.17.1-v1\n")?;
    eprintln!("Verified NVIDIA runtime cached in {}", root.display());
    activate_cached()
}

#[cfg(test)]
mod tests {
    use super::{ARCHIVES, EXPECTED, hash_output};
    #[test]
    fn parses_windows_certutil_output() {
        let digest = "c8fb12715b9639f51983315cc4b195e272128c675aa3766b13c4f470b892e6c8";
        assert_eq!(hash_output(&format!("SHA256 hash of file X:\n{digest}\nCertUtil: success")), Some(digest.to_string()));
        assert!(hash_output("SHA256 hash of file X:\nINVALID").is_none());
    }
    #[test]
    fn all_gpu_packages_are_checksum_pinned() {
        assert_eq!(ARCHIVES.len(), 6);
        for a in &ARCHIVES {
            assert!(a.path.ends_with(".zip"));
            assert_eq!(a.digest.len(), 64);
            assert!(a.digest.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(a.size > 1_000_000);
        }
        assert!(EXPECTED.contains(&"cublasLt64_13.dll"));
    }
}
