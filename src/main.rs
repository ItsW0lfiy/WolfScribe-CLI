use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;
use backend::Device;
use parakeet_rs::{ParakeetTDT, TimestampMode, Transcriber};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::Instant;

mod backend;
#[cfg(all(windows, feature = "cuda"))]
mod cuda_runtime;
#[cfg(all(windows, feature = "cuda"))]
mod ort_runtime;
mod ffmpeg;
mod model_download;

const SAMPLE_RATE: usize = 16_000;

/// Transcribe a media file using Parakeet TDT v3. Requires FFmpeg on PATH.
#[derive(Debug, Parser)]
#[command(name = "wolfscribe", version, about)]
struct Args {
    /// Input files or directories. Directories include supported media files (not recursive).
    #[arg(value_name = "INPUT", required = true, num_args = 1..)]
    inputs: Vec<PathBuf>,

    /// Optional model directory override. Defaults to per-user application data.
    #[arg(long)]
    model_dir: Option<PathBuf>,

    /// Never download model files. Fail when required files are not installed.
    #[arg(long)]
    offline: bool,

    /// Inference backend: auto tries CUDA in CUDA-enabled builds, then CPU.
    #[arg(long, value_enum, default_value_t = Device::Auto)]
    device: Device,

    /// Authorize NVIDIA runtime download only if legally eligible and licenses accepted.
    #[arg(long)]
    accept_nvidia_license: bool,

    /// Number of seconds assigned to each segment.
    #[arg(long, default_value_t = 30)]
    chunk_seconds: usize,

    /// Extra seconds decoded before and after each segment.
    #[arg(long, default_value_t = 2)]
    context_seconds: usize,
}

/// PCM samples decoded from FFmpeg stdout, retaining only a bounded window.
struct PcmStream {
    child: Child,
    stdout: ChildStdout,
    samples: Vec<f32>,
    buffer_start: usize,
    trailing_bytes: Vec<u8>,
    eof: bool,
}

impl PcmStream {
    fn open(input: &PathBuf) -> Result<Self> {
        let mut child = Command::new(ffmpeg::find_ffmpeg())
            .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
            .arg(input)
            .args([
                "-map",
                "0:a:0",
                "-vn",
                "-sn",
                "-dn",
                "-ac",
                "1",
                "-ar",
                "16000",
                "-f",
                "f32le",
                "-acodec",
                "pcm_f32le",
                "pipe:1",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("Could not launch FFmpeg. Is ffmpeg.exe on PATH?")?;

        let stdout = child
            .stdout
            .take()
            .context("FFmpeg stdout was unavailable")?;
        Ok(Self {
            child,
            stdout,
            samples: Vec::new(),
            buffer_start: 0,
            trailing_bytes: Vec::new(),
            eof: false,
        })
    }

    /// Fill until an absolute sample index is available, or FFmpeg ends.
    fn fill_until(&mut self, requested_end: usize) -> Result<()> {
        let mut bytes = [0_u8; 64 * 1024];
        while !self.eof && self.buffer_start + self.samples.len() < requested_end {
            let count = self
                .stdout
                .read(&mut bytes)
                .context("Reading FFmpeg PCM output")?;
            if count == 0 {
                self.eof = true;
                if !self.trailing_bytes.is_empty() {
                    bail!("Incomplete float32 sample from FFmpeg");
                }
                break;
            }
            // stdout reads are not guaranteed to end at a 4-byte float boundary.
            let mut complete = std::mem::take(&mut self.trailing_bytes);
            complete.extend_from_slice(&bytes[..count]);
            let whole = complete.len() / 4 * 4;
            for four in complete[..whole].chunks_exact(4) {
                self.samples
                    .push(f32::from_le_bytes([four[0], four[1], four[2], four[3]]));
            }
            self.trailing_bytes.extend_from_slice(&complete[whole..]);
        }
        Ok(())
    }

    fn decoded_end(&self) -> usize {
        self.buffer_start + self.samples.len()
    }

    fn discard_before(&mut self, absolute_sample: usize) {
        let drain_count = absolute_sample
            .saturating_sub(self.buffer_start)
            .min(self.samples.len());
        self.samples.drain(..drain_count);
        self.buffer_start += drain_count;
    }

    fn finish(mut self) -> Result<()> {
        // Ensure the pipe reaches EOF and FFmpeg's exit status is checked.
        let mut sink = [0_u8; 64 * 1024];
        while self
            .stdout
            .read(&mut sink)
            .context("Draining FFmpeg stdout")?
            != 0
        {}
        let status = self.child.wait().context("Waiting for FFmpeg")?;
        if !status.success() {
            bail!("FFmpeg failed with status {status}");
        }
        Ok(())
    }
}

impl Drop for PcmStream {
    fn drop(&mut self) {
        // On inference failure, don't leave a blocked ffmpeg.exe behind.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn srt_time(seconds: f64) -> String {
    let total_ms = (seconds.max(0.0) * 1000.0).round() as u64;
    let hours = total_ms / 3_600_000;
    let minutes = (total_ms / 60_000) % 60;
    let secs = (total_ms / 1000) % 60;
    let millis = total_ms % 1000;
    format!("{hours:02}:{minutes:02}:{secs:02},{millis:03}")
}

/// Resolve model weights without requiring the old benchmark folder.
fn resolve_model_dir(explicit: Option<PathBuf>, offline: bool) -> Result<PathBuf> {
    let user_supplied = explicit.is_some();
    let directory = if let Some(path) = explicit {
        path
    } else if let Some(value) =
        std::env::var_os("WOLFSCRIBE_MODEL_DIR").filter(|value| !value.is_empty())
    {
        PathBuf::from(value)
    } else if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        PathBuf::from(local)
            .join("WolfScribe")
            .join("models")
            .join("parakeet-tdt-v3")
    } else {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|home| PathBuf::from(home).join(".local").join("share"))
            })
            .context("No model directory found; set WOLFSCRIBE_MODEL_DIR or pass --model-dir")?;
        base.join("wolfscribe").join("models").join("parakeet-tdt-v3")
    };

    model_download::ensure_model_available(&directory, !user_supplied && !offline)?;
    Ok(directory)
}

/// Enumerate positional files and immediate children of directories exactly once.
/// Reject output collisions before opening any output file.
fn expand_inputs(inputs: &[PathBuf]) -> Result<Vec<PathBuf>> {
    const MEDIA: [&str; 17] = [
        "aac", "avi", "flac", "m4a", "mka", "mkv", "mov", "mp3", "mp4",
        "mpeg", "mpg", "ogg", "opus", "ts", "wav", "webm", "wma",
    ];
    let mut selected = BTreeSet::new();
    for input in inputs {
        if input.is_file() {
            selected.insert(input.canonicalize().with_context(|| format!("Resolve {}", input.display()))?);
        } else if input.is_dir() {
            let mut included = 0;
            for entry in fs::read_dir(input).with_context(|| format!("Read directory {}", input.display()))? {
                let path = entry?.path();
                if path.is_file()
                    && path.extension().and_then(|ext| ext.to_str())
                        .is_some_and(|ext| MEDIA.contains(&ext.to_ascii_lowercase().as_str()))
                {
                    selected.insert(path.canonicalize()?);
                    included += 1;
                }
            }
            if included == 0 {
                bail!("Directory contains no supported media files: {}", input.display());
            }
        } else {
            bail!("Input is not a file or directory: {}", input.display());
        }
    }
    if selected.is_empty() {
        bail!("No input files provided");
    }
    let mut outputs = BTreeMap::<PathBuf, PathBuf>::new();
    for input in &selected {
        let srt = input.with_extension("srt");
        if let Some(other) = outputs.insert(srt.clone(), input.clone()) {
            bail!(
                "Output name conflict: {} and {} would both write {}. Rename one input.",
                other.display(), input.display(), srt.display()
            );
        }
    }
    Ok(selected.into_iter().collect())
}

/// Reuse a single loaded Parakeet session for every file in a batch.
/// Audio state and subtitle timestamps always reset between input files.
fn transcribe_file(
    input_path: &PathBuf,
    model_dir: &std::path::Path,
    model: &mut Option<ParakeetTDT>,
    using_cuda: &mut bool,
    args: &Args,
) -> Result<()> {
    let started = Instant::now();
    let chunk_size = args.chunk_seconds * SAMPLE_RATE;
    let context_size = args.context_seconds * SAMPLE_RATE;
    let mut input = PcmStream::open(input_path)?;
    let srt_path = input_path.with_extension("srt");
    let txt_path = input_path.with_extension("txt");
    let mut srt = BufWriter::new(
        File::create(&srt_path).with_context(|| format!("Creating {}", srt_path.display()))?,
    );
    let mut txt = BufWriter::new(
        File::create(&txt_path).with_context(|| format!("Creating {}", txt_path.display()))?,
    );

    let mut core_start = 0_usize;
    let mut caption_index = 0_usize;
    let mut segments_processed = 0_usize;

    loop {
        // Each inference receives its 30s main interval plus 2s of context on each side.
        // All audio stays in RAM; no intermediate media files are created.
        let window_start = core_start.saturating_sub(context_size);
        let requested_end = core_start + chunk_size + context_size;
        input.fill_until(requested_end)?;

        if input.decoded_end() <= core_start {
            break;
        }
        let window_end = input.decoded_end().min(requested_end);
        let first = window_start
            .checked_sub(input.buffer_start)
            .context("Internal buffer underflow")?;
        let last = window_end - input.buffer_start;
        let samples = input.samples[first..last].to_vec();

        // CUDA may successfully register its execution provider but fail on
        // the first actual kernel launch (e.g. unsupported Blackwell SM). In
        // auto mode, retry *this same chunk* on CPU. Already completed chunks
        // remain on disk, so no transcript content needs to be discarded.
        let mode = Some(TimestampMode::Sentences);
        let result = if backend::should_fallback_on_inference_error(args.device, *using_cuda) {
            match model.as_mut().expect("Model is loaded").transcribe_samples(samples.clone(), SAMPLE_RATE as u32, 1, mode) {
                Ok(result) => result,
                Err(error) => {
                    eprintln!(
                        "CUDA inference failed near {:.1}s: {error}",
                        core_start as f64 / SAMPLE_RATE as f64
                    );
                    eprintln!("{}", backend::summarize_gpu_failure(&error.to_string()));
                    eprintln!("Switching to CPU and retrying the current audio chunk...");
                    // Release GPU sessions before allocating CPU model weights.
                    drop(model.take());
                    let fallback = backend::load_model(&model_dir, Device::Cpu, true, false)
                        .context("GPU inference failed, and CPU fallback could not load the model")?;
                    *model = Some(fallback.model);
                    *using_cuda = false;
                    eprintln!("Backend: CPU (automatic inference fallback)");
                    model.as_mut().expect("CPU fallback is loaded")
                        .transcribe_samples(samples, SAMPLE_RATE as u32, 1, mode)
                        .map_err(|cpu_error| {
                            anyhow!(
                                "CPU transcription failed near {:.1}s after CUDA fallback: {cpu_error}",
                                core_start as f64 / SAMPLE_RATE as f64
                            )
                        })?
                }
            }
        } else {
            model.as_mut().expect("Model is loaded")
                .transcribe_samples(samples, SAMPLE_RATE as u32, 1, mode)
                .map_err(|e| {
                    anyhow!(
                        "Transcription failed near {:.1}s: {e}",
                        core_start as f64 / SAMPLE_RATE as f64
                    )
                })?
        };

        // A token belongs to the 30s core interval containing its midpoint.
        // Context helps with words at boundaries; midpoint ownership prevents most duplicates.
        // Sentence splitting/alignment can be improved in a future version.
        let core_start_s = core_start as f64 / SAMPLE_RATE as f64;
        let core_end_s =
            (core_start + chunk_size).min(input.decoded_end()) as f64 / SAMPLE_RATE as f64;
        let window_start_s = window_start as f64 / SAMPLE_RATE as f64;

        for token in &result.tokens {
            let speech = token.text.trim();
            if speech.is_empty() {
                continue;
            }
            let start = window_start_s + token.start as f64;
            let end = window_start_s + token.end as f64;
            if !start.is_finite() || !end.is_finite() || end <= start {
                continue;
            }
            let midpoint = (start + end) / 2.0;
            if midpoint < core_start_s || midpoint >= core_end_s {
                continue;
            }

            caption_index += 1;
            writeln!(
                srt,
                "{caption_index}\n{} --> {}\n{speech}\n",
                srt_time(start),
                srt_time(end)
            )?;
            writeln!(txt, "{speech}")?;
        }
        srt.flush()?;
        txt.flush()?;

        segments_processed += 1;
        println!(
            "Processed through {:>7.1}s | {} segments | {} captions | elapsed {:.1}s",
            core_end_s,
            segments_processed,
            caption_index,
            started.elapsed().as_secs_f64()
        );

        if input.eof && input.decoded_end() <= core_start + chunk_size {
            break;
        }
        core_start += chunk_size;
        input.discard_before(core_start.saturating_sub(context_size));
    }

    // Check for failed decoding, including files without a usable audio stream.
    input.finish()?;
    srt.flush()?;
    txt.flush()?;
    println!("\nFile completed in {:.2}s", started.elapsed().as_secs_f64());
    println!("SRT: {}", srt_path.display());
    println!("TXT: {}", txt_path.display());
    Ok(())
}
}

fn main() -> Result<()> {
    let args = Args::parse();
    if !(10..=120).contains(&args.chunk_seconds) {
        bail!("--chunk-seconds must be 10 through 120");
    }
    if args.context_seconds >= args.chunk_seconds / 2 {
        bail!("--context-seconds must be less than half --chunk-seconds");
    }
    let inputs = expand_inputs(&args.inputs)?;
    let model_dir = resolve_model_dir(args.model_dir.clone(), args.offline)?;
    let total_started = Instant::now();

    #[cfg(all(windows, feature = "cuda"))]
    ort_runtime::initialize(args.device, args.offline)?;

    println!("Loading Parakeet TDT from {}", model_dir.display());
    let loaded = backend::load_model(&model_dir, args.device, args.offline, args.accept_nvidia_license)?;
    println!("Backend: {}", loaded.backend);
    let mut using_cuda = loaded.uses_cuda;
    let mut model = Some(loaded.model);
    println!("Model loaded in {:.2}s", total_started.elapsed().as_secs_f64());
    println!("Processing {} input file(s) with one loaded model", inputs.len());

    let mut failures = 0usize;
    for (index, input_path) in inputs.iter().enumerate() {
        println!("\n[{}/{}] {}", index + 1, inputs.len(), input_path.display());
        if let Err(error) = transcribe_file(input_path, &model_dir, &mut model, &mut using_cuda, &args) {
            failures += 1;
            eprintln!("Failed {}: {error:#}", input_path.display());
        }
    }
    println!(
        "\nBatch completed: {} succeeded, {} failed, total {:.2}s",
        inputs.len() - failures, failures, total_started.elapsed().as_secs_f64()
    );
    if failures > 0 {
        bail!("{failures} input file(s) failed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Args, Device, expand_inputs, srt_time};
    use std::fs;
    use clap::Parser;

    #[test]
    fn accepts_requested_device() {
        let args = Args::try_parse_from(["wolfscribe", "audio.wav", "--device", "cuda"]).unwrap();
        assert_eq!(args.device, Device::Cuda);
        let defaults = Args::try_parse_from(["wolfscribe", "audio.wav"]).unwrap();
        assert_eq!(defaults.device, Device::Auto);
        assert_eq!(defaults.inputs.len(), 1);
        let batch = Args::try_parse_from(["wolfscribe", "first.webm", "second.mp3", "--device", "auto"]).unwrap();
        assert_eq!(batch.inputs.len(), 2);
    }

    #[test]
    fn batch_directory_and_duplicate_inputs() {
        let dir = std::env::temp_dir().join(format!("wolfscribe-batch-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let a = dir.join("audio.wav");
        let b = dir.join("other.webm");
        let ignored = dir.join("notes.txt");
        for f in [&a, &b, &ignored] { fs::write(f, b"test").unwrap(); }
        let inputs = expand_inputs(&[dir.clone(), a.clone()]).unwrap();
        assert_eq!(inputs.len(), 2);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_colliding_subtitle_names() {
        let dir = std::env::temp_dir().join(format!("wolfscribe-collision-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let a = dir.join("same.mp3");
        let b = dir.join("same.webm");
        fs::write(&a, b"test").unwrap();
        fs::write(&b, b"test").unwrap();
        let result = expand_inputs(&[a, b]);
        assert!(result.is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn formats_subtitle_timestamps() {
        assert_eq!(srt_time(0.0), "00:00:00,000");
        assert_eq!(srt_time(61.125), "00:01:01,125");
        assert_eq!(srt_time(3601.999), "01:00:01,999");
    }
}
