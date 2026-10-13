use std::ffi::OsString;
use std::path::PathBuf;

/// Find an adjacent bundled FFmpeg before consulting PATH. Keep dev-only
/// tools/ support so a repository checkout can test bundling locally.
pub(crate) fn find_ffmpeg() -> OsString {
    let name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    let candidate = std::env::current_exe().ok().and_then(|exe| {
        let adjacent = exe.parent()?.join(name);
        adjacent.is_file().then_some(adjacent)
    });
    if let Some(candidate) = candidate {
        return candidate.into_os_string();
    }

    if let Ok(cwd) = std::env::current_dir() {
        let dev = cwd.join("tools").join(name);
        if dev.is_file() {
            return dev.into_os_string();
        }
    }

    OsString::from("ffmpeg")
}

/// For diagnostics and tests without relying on any FFmpeg installation.
#[cfg(test)]
pub(crate) fn choose_ffmpeg(exe_dir: PathBuf, cwd: PathBuf, name: &str) -> PathBuf {
    let bundled = exe_dir.join(name);
    if bundled.is_file() {
        return bundled;
    }
    let dev = cwd.join("tools").join(name);
    if dev.is_file() {
        return dev;
    }
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::choose_ffmpeg;
    use std::{fs, path::PathBuf};

    #[test]
    fn falls_back_to_path_without_local_binary() {
        let nonexistent = PathBuf::from("/unavailable/wolfscribe-test-ffmpeg");
        assert_eq!(
            choose_ffmpeg(nonexistent.clone(), nonexistent, "ffmpeg.exe"),
            PathBuf::from("ffmpeg.exe")
        );
    }

    #[test]
    fn chooses_bundled_executable_before_developer_tool() {
        let base = std::env::temp_dir().join(format!("wolfscribe-ffmpeg-test-{}", std::process::id()));
        let exe = base.join("exe");
        let cwd = base.join("work");
        fs::create_dir_all(&exe).unwrap();
        fs::create_dir_all(cwd.join("tools")).unwrap();
        let binary = exe.join("ffmpeg.exe");
        fs::write(&binary, b"test").unwrap();
        fs::write(cwd.join("tools").join("ffmpeg.exe"), b"test").unwrap();
        assert_eq!(choose_ffmpeg(exe, cwd, "ffmpeg.exe"), binary);
        fs::remove_dir_all(base).unwrap();
    }
}
