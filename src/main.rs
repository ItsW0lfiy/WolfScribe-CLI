use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;
use parakeet_rs::{ParakeetTDT, TimestampMode, Transcriber};
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::Instant;

const SAMPLE_RATE: usize = 16_000;

/// Transcribe a media file using Parakeet TDT v3. Requires FFmpeg on PATH.
#[derive(Debug, Parser)]
#[command(version, about)]
struct Args {
    /// Input audio or video file (any format FFmpeg can decode).
    input: PathBuf,

    /// Directory holding encoder-model.onnx, decoder_joint-model.onnx, etc.
    #[arg(long)]
    model_dir: PathBuf,

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
        let mut child = Command::new("ffmpeg")
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

fn main() -> Result<()> {
    let args = Args::parse();
    if !args.input.is_file() {
        bail!("Input file does not exist: {}", args.input.display());
    }
    if !args.model_dir.is_dir() {
        bail!(
            "Model directory does not exist: {}",
            args.model_dir.display()
        );
    }
    if !(10..=120).contains(&args.chunk_seconds) {
        bail!("--chunk-seconds must be 10 through 120");
    }
    if args.context_seconds >= args.chunk_seconds / 2 {
        bail!("--context-seconds must be less than half --chunk-seconds");
    }

    let started = Instant::now();
    let chunk_size = args.chunk_seconds * SAMPLE_RATE;
    let context_size = args.context_seconds * SAMPLE_RATE;

    println!("Loading Parakeet TDT from {}", args.model_dir.display());
    let mut model = ParakeetTDT::from_pretrained(&args.model_dir, None)
        .map_err(|e| anyhow!("Failed loading Parakeet: {e}"))?;
    println!("Model loaded in {:.2}s", started.elapsed().as_secs_f64());

    let mut input = PcmStream::open(&args.input)?;
    let srt_path = args.input.with_extension("srt");
    let txt_path = args.input.with_extension("txt");
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

        let result = model
            .transcribe_samples(
                samples,
                SAMPLE_RATE as u32,
                1,
                Some(TimestampMode::Sentences),
            )
            .map_err(|e| {
                anyhow!(
                    "Transcription failed near {:.1}s: {e}",
                    core_start as f64 / SAMPLE_RATE as f64
                )
            })?;

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
    println!("\nDone in {:.2}s", started.elapsed().as_secs_f64());
    println!("SRT: {}", srt_path.display());
    println!("TXT: {}", txt_path.display());
    Ok(())
}
