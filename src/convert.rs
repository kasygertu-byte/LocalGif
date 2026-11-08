//! Converts GIFs that are really WebP files into real GIFs before they are copied.
//!
//! Discord often saves "GIFs" as animated WebP (sometimes still named `.gif`). The
//! upload preview plays those, but Discord can turn them into a still image once
//! sent. Pasting a real GIF avoids that.

use image::codecs::gif::{GifEncoder, Repeat};
use image::codecs::webp::WebPDecoder;
use image::{AnimationDecoder, ImageFormat};
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};

/// True if the file's content is not GIF (checked by content, not by name).
pub fn needs_conversion(path: &Path) -> bool {
    image::ImageReader::open(path)
        .and_then(|r| r.with_guessed_format())
        .is_ok_and(|r| r.format() == Some(ImageFormat::WebP))
}

/// Converts `src` to a GIF in the cache folder and returns its path.
/// Reuses an earlier conversion if the source file hasn't changed.
pub fn to_gif(src: &Path) -> Result<PathBuf, String> {
    let meta = fs::metadata(src).map_err(|e| e.to_string())?;
    // One folder per source version, so the GIF keeps the original's name.
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    src.hash(&mut hasher);
    meta.len().hash(&mut hasher);
    meta.modified().ok().hash(&mut hasher);
    let dir = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("LocalGif")
        .join("converted")
        .join(format!("{:016x}", hasher.finish()));
    let stem = src
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "gif".into());
    let dest = dir.join(format!("{stem}.gif"));
    if dest.exists() {
        return Ok(dest);
    }
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let err = |e: image::ImageError| e.to_string();
    let decoder = WebPDecoder::new(BufReader::new(File::open(src).map_err(|e| e.to_string())?))
        .map_err(err)?;
    let tmp = dir.join(format!("{stem}.gif.part"));
    {
        let out = BufWriter::new(File::create(&tmp).map_err(|e| e.to_string())?);
        // Speed 10 (of 1-30): good colors without taking long.
        let mut encoder = GifEncoder::new_with_speed(out, 10);
        encoder.set_repeat(Repeat::Infinite).map_err(err)?;
        if decoder.has_animation() {
            // Frame by frame, so large GIFs don't need all frames in memory at once.
            for frame in decoder.into_frames() {
                encoder.encode_frame(frame.map_err(err)?).map_err(err)?;
            }
        } else {
            let still = image::DynamicImage::from_decoder(decoder).map_err(err)?;
            encoder
                .encode_frame(image::Frame::new(still.into_rgba8()))
                .map_err(err)?;
        }
    }
    fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
    Ok(dest)
}
