//! Single owner of the GPU: runs engine jobs one at a time and batches whatever
//! queued up meanwhile.
//!
//! Every engine run initialises Vulkan and loads the model, which for a typical
//! manga page costs about as much as the inference itself, and parallel runs
//! only fight over VRAM. So one worker thread takes the oldest job, sweeps in
//! the other queued jobs with the same engine settings, and processes them in a
//! single directory-mode run. A lone request starts immediately; only work that
//! would have waited for the GPU anyway gets grouped.

use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc;
#[cfg(all(test, unix))]
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::oneshot;

use crate::upscaler::Upscaler;

/// Upper bound on images per engine run, to cap temp disk and memory use.
const MAX_BATCH: usize = 8;

/// Engine output for one job.
pub struct Processed {
    pub bytes: Vec<u8>,
    /// Engine time attributed to this job: a shared run's wall time divided
    /// evenly among its images. Excludes time spent waiting in the queue.
    pub engine_ms: u64,
}

struct Job {
    upscaler: Upscaler,
    input: Vec<u8>,
    reply: oneshot::Sender<Result<Processed, String>>,
}

pub struct GpuQueue {
    tx: mpsc::Sender<Job>,
}

impl GpuQueue {
    /// Start the worker thread. It lives as long as the queue.
    pub fn spawn() -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("gpu-queue".to_string())
            .spawn(move || worker(rx))
            .expect("failed to spawn the GPU queue thread");
        Self { tx }
    }

    /// Queue an image for upscaling with `upscaler`'s settings and wait for it.
    pub async fn submit(&self, upscaler: Upscaler, input: Vec<u8>) -> Result<Processed, String> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Job { upscaler, input, reply })
            .map_err(|_| "GPU worker is not running".to_string())?;
        rx.await.unwrap_or_else(|_| Err("Worker thread failed".to_string()))
    }
}

fn worker(rx: mpsc::Receiver<Job>) {
    let mut backlog: VecDeque<Job> = VecDeque::new();
    loop {
        backlog.extend(rx.try_iter());
        // Requests whose client already went away (e.g. the reader scrolled
        // past the page) aren't worth GPU time.
        backlog.retain(|job| !job.reply.is_closed());
        let first = match backlog.pop_front() {
            Some(job) => job,
            None => match rx.recv() {
                Ok(job) => job,
                Err(_) => return,
            },
        };

        let key = first.upscaler.engine_key();
        let mut batch = vec![first];
        let mut rest = VecDeque::with_capacity(backlog.len());
        for job in backlog.drain(..) {
            if batch.len() < MAX_BATCH && job.upscaler.engine_key() == key {
                batch.push(job);
            } else {
                rest.push_back(job);
            }
        }
        backlog = rest;

        // A panic drops this batch's reply senders, which their callers see as
        // a failed job; the worker itself keeps serving.
        let _ = catch_unwind(AssertUnwindSafe(|| run(batch)));
    }
}

fn run(batch: Vec<Job>) {
    let start = Instant::now();
    let inputs: Vec<&[u8]> = batch.iter().map(|job| job.input.as_slice()).collect();
    let shared = batch[0].upscaler.process_batch(&inputs);
    let share_ms = start.elapsed().as_millis() as u64 / batch.len() as u64;

    match shared {
        Ok(outputs) => {
            for (job, output) in batch.into_iter().zip(outputs) {
                let _ = job.reply.send(output.map(|bytes| Processed {
                    bytes,
                    engine_ms: share_ms,
                }));
            }
        }
        // The shared run failed as a whole; retry singly so one input that
        // crashes the engine can't take its neighbours down with it.
        Err(_) if batch.len() > 1 => {
            for job in batch {
                let start = Instant::now();
                let output = job.upscaler.process(&job.input);
                let engine_ms = start.elapsed().as_millis() as u64;
                let _ = job.reply.send(output.map(|bytes| Processed { bytes, engine_ms }));
            }
        }
        Err(e) => {
            for job in batch {
                let _ = job.reply.send(Err(e.clone()));
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::time::Duration;

    /// A stand-in engine: copies `-i` to `-o` (file or directory mode) after a
    /// short delay, and appends one line per invocation to `calls.log`.
    const FAKE_ENGINE: &str = r#"#!/bin/sh
while [ $# -gt 0 ]; do
  case "$1" in -i) in="$2"; shift 2;; -o) out="$2"; shift 2;; *) shift;; esac
done
dir=$(dirname "$0")
echo run >> "$dir/calls.log"
sleep 0.3
if [ -d "$in" ]; then cp "$in"/* "$out"/; else cp "$in" "$out"; fi
"#;

    struct Fixture {
        base: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let base = std::env::temp_dir().join(format!(
                "miko-kagura-gpu-queue-test-{}-{nanos}",
                std::process::id()
            ));
            let engine_dir = base.join("realcugan");
            std::fs::create_dir_all(&engine_dir).expect("create fixture dir");
            let engine = engine_dir.join("realcugan-ncnn-vulkan");
            std::fs::write(&engine, FAKE_ENGINE).expect("write fake engine");
            std::fs::set_permissions(&engine, std::fs::Permissions::from_mode(0o755))
                .expect("make fake engine executable");
            Fixture { base }
        }

        fn upscaler(&self) -> Upscaler {
            let mut up = Upscaler::new(self.base.to_str().unwrap(), -1);
            up.load_model("realcugan-se", 2, 0).expect("load model");
            up
        }

        fn calls(&self) -> usize {
            std::fs::read_to_string(self.base.join("realcugan").join("calls.log"))
                .map(|s| s.lines().count())
                .unwrap_or(0)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.base);
        }
    }

    fn png(width: u32) -> Vec<u8> {
        let img = image::RgbImage::new(width, 1);
        let mut buf = Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        buf.into_inner()
    }

    fn width_of(bytes: &[u8]) -> u32 {
        image::load_from_memory(bytes).unwrap().width()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn jobs_queued_behind_a_running_one_share_one_engine_run() {
        let fx = Fixture::new();
        let queue = Arc::new(GpuQueue::spawn());

        let first = {
            let (queue, up) = (queue.clone(), fx.upscaler());
            tokio::spawn(async move { queue.submit(up, png(1)).await })
        };
        // Let the first job reach the engine, then queue three more behind it.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let rest: Vec<_> = (2..=4)
            .map(|w| {
                let (queue, up) = (queue.clone(), fx.upscaler());
                tokio::spawn(async move { queue.submit(up, png(w)).await })
            })
            .collect();

        assert_eq!(width_of(&first.await.unwrap().unwrap().bytes), 1);
        for (w, job) in (2..=4).zip(rest) {
            assert_eq!(width_of(&job.await.unwrap().unwrap().bytes), w);
        }
        assert_eq!(fx.calls(), 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_undecodable_input_fails_alone() {
        let fx = Fixture::new();
        let up = fx.upscaler();
        let good = png(3);
        let results = up.process_batch(&[b"not an image", &good]).unwrap();
        assert!(results[0].as_ref().unwrap_err().contains("decode"));
        assert_eq!(width_of(results[1].as_ref().unwrap()), 3);
        assert_eq!(fx.calls(), 1);
    }

    /// Times real engines from `MIKO_ENGINE_DIR`: four separate runs against
    /// one shared run over the same four pages.
    /// `MIKO_ENGINE_DIR=… cargo test --release engine_benchmark -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn engine_benchmark() {
        let dir = std::env::var("MIKO_ENGINE_DIR").expect("MIKO_ENGINE_DIR");
        let page = |seed: u32| {
            let img = image::RgbImage::from_fn(900, 1300, |x, y| {
                let v = ((x * 7 + y * 13 + seed * 31) % 251) as u8;
                image::Rgb([v, v.wrapping_mul(3), 255 - v])
            });
            let mut buf = Cursor::new(Vec::new());
            img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
            buf.into_inner()
        };
        let pages: Vec<Vec<u8>> = (0..4).map(page).collect();
        let refs: Vec<&[u8]> = pages.iter().map(Vec::as_slice).collect();

        for model in ["realcugan-se", "waifu2x", "realesrgan-anime"] {
            let config = crate::config::get_model(model).unwrap();
            let mut up = Upscaler::new(&dir, -1);
            up.detect_gpu();
            up.load_model(model, config.supported_scales[0], config.supported_noise[0])
                .unwrap();

            let start = std::time::Instant::now();
            for p in &refs {
                up.process(p).expect("single run");
            }
            let single = start.elapsed();

            let start = std::time::Instant::now();
            let outputs = up.process_batch(&refs).expect("shared run");
            let shared = start.elapsed();
            for out in &outputs {
                let w = width_of(out.as_ref().expect("batch item"));
                assert_eq!(w, 900 * up.current_scale as u32);
            }
            println!(
                "{model} x{} on {}: 4 single runs {single:.2?}, one shared run {shared:.2?}",
                up.current_scale, up.gpu_name
            );
        }
    }
}
