//! Background GIF decoding.
//!
//! Decoding happens on worker threads so scrolling stays smooth. Frames are
//! downscaled to thumbnail size to keep memory use reasonable.

use eframe::egui;
use image::codecs::gif::GifDecoder;
use image::codecs::webp::WebPDecoder;
use image::{AnimationDecoder, ImageFormat};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};

/// Largest width/height (in pixels) a decoded frame is scaled down to.
const MAX_THUMB_SIZE: u32 = 320;

pub struct Frame {
    pub image: egui::ColorImage,
    /// How long the frame is shown, in seconds.
    pub delay: f64,
}

pub struct DecodedGif {
    pub frames: Vec<Frame>,
    /// Tiny version of the first frame. Stretched to tile size it looks heavily
    /// blurred, which is used as the censored preview of NSFW GIFs.
    pub censored: egui::ColorImage,
}

pub struct Decoded {
    pub id: u64,
    pub result: Result<DecodedGif, String>,
}

struct Job {
    id: u64,
    path: PathBuf,
}

struct Shared {
    queue: Mutex<Vec<Job>>,
    cv: Condvar,
}

pub struct Loader {
    shared: Arc<Shared>,
    rx: Receiver<Decoded>,
}

impl Loader {
    pub fn new(ctx: egui::Context) -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Vec::new()),
            cv: Condvar::new(),
        });
        let (tx, rx) = channel();
        let workers = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2)
            .clamp(1, 4);
        for _ in 0..workers {
            let shared = shared.clone();
            let tx = tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || worker(shared, tx, ctx));
        }
        Self { shared, rx }
    }

    pub fn request(&self, id: u64, path: PathBuf) {
        let mut q = self.shared.queue.lock().unwrap();
        if !q.iter().any(|j| j.id == id) {
            q.push(Job { id, path });
            self.shared.cv.notify_one();
        }
    }

    /// Drops queued (not yet started) jobs for which `keep` returns false.
    pub fn retain(&self, mut keep: impl FnMut(u64) -> bool) {
        self.shared.queue.lock().unwrap().retain(|j| keep(j.id));
    }

    pub fn try_recv(&self) -> Option<Decoded> {
        self.rx.try_recv().ok()
    }
}

fn worker(shared: Arc<Shared>, tx: Sender<Decoded>, ctx: egui::Context) {
    loop {
        let job = {
            let mut q = shared.queue.lock().unwrap();
            loop {
                // LIFO: the most recently requested (= currently visible) GIFs first.
                if let Some(job) = q.pop() {
                    break job;
                }
                q = shared.cv.wait(q).unwrap();
            }
        };
        let result = decode(&job.path);
        if tx.send(Decoded { id: job.id, result }).is_err() {
            return; // App closed.
        }
        ctx.request_repaint();
    }
}

fn decode(path: &Path) -> Result<DecodedGif, String> {
    let err = |e: image::ImageError| e.to_string();
    let format = image::ImageReader::open(path)
        .and_then(|r| r.with_guessed_format())
        .map_err(|e| e.to_string())?
        .format();
    let reader = BufReader::new(File::open(path).map_err(|e| e.to_string())?);

    // Decode by actual content: files named .gif can contain WebP data.
    let raw_frames: Vec<image::Frame> = match format {
        Some(ImageFormat::WebP) => {
            let decoder = WebPDecoder::new(reader).map_err(err)?;
            if decoder.has_animation() {
                decoder
                    .into_frames()
                    .collect::<Result<_, _>>()
                    .map_err(err)?
            } else {
                let still = image::DynamicImage::from_decoder(decoder).map_err(err)?;
                vec![image::Frame::new(still.into_rgba8())]
            }
        }
        _ => GifDecoder::new(reader)
            .map_err(err)?
            .into_frames()
            .collect::<Result<_, _>>()
            .map_err(err)?,
    };

    let mut frames = Vec::new();
    let mut censored = None;
    for frame in raw_frames {
        let (num, den) = frame.delay().numer_denom_ms();
        let mut ms = if den == 0 {
            0.0
        } else {
            num as f64 / den as f64
        };
        // Same rule browsers use: very small delays are treated as 100 ms.
        if ms <= 10.0 {
            ms = 100.0;
        }

        let mut buf = frame.into_buffer();
        let (w, h) = buf.dimensions();
        if w > MAX_THUMB_SIZE || h > MAX_THUMB_SIZE {
            let scale = MAX_THUMB_SIZE as f32 / w.max(h) as f32;
            let nw = ((w as f32 * scale).round() as u32).max(1);
            let nh = ((h as f32 * scale).round() as u32).max(1);
            buf = image::imageops::resize(&buf, nw, nh, image::imageops::FilterType::Triangle);
        }
        let (w, h) = buf.dimensions();
        if censored.is_none() {
            censored = Some(censor(&buf));
        }
        frames.push(Frame {
            image: egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], buf.as_raw()),
            delay: ms / 1000.0,
        });
    }

    match censored {
        Some(censored) => Ok(DecodedGif { frames, censored }),
        None => Err("GIF has no frames".into()),
    }
}

/// Shrinks a frame to at most 8 px so no details remain.
fn censor(buf: &image::RgbaImage) -> egui::ColorImage {
    const SIZE: f32 = 8.0;
    let (w, h) = buf.dimensions();
    let scale = SIZE / w.max(h) as f32;
    let nw = ((w as f32 * scale).round() as u32).max(1);
    let nh = ((h as f32 * scale).round() as u32).max(1);
    let small = image::imageops::resize(buf, nw, nh, image::imageops::FilterType::Triangle);
    egui::ColorImage::from_rgba_unmultiplied([nw as usize, nh as usize], small.as_raw())
}
