//! GIF packs: several GIFs plus their labels, tags and flags in one `.gifpack` file,
//! for moving them to another device.
//!
//! A pack is an ordinary ZIP archive with **no compression** ("stored"), so every
//! GIF is copied byte-for-byte and loses nothing:
//!
//! ```text
//! pack.json          {"format": "localgif-pack", "version": 1, "gifs": [...]}
//! gifs/<file name>   the GIF files, unchanged
//! ```

use crate::library::PackMeta;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

pub const EXTENSION: &str = "gifpack";
const FORMAT: &str = "localgif-pack";
const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Manifest {
    format: String,
    version: u32,
    gifs: Vec<PackGif>,
}

#[derive(Serialize, Deserialize)]
struct PackGif {
    /// File name inside `gifs/`.
    file: String,
    #[serde(flatten)]
    meta: PackMeta,
}

/// One GIF to put in a pack.
pub struct ExportItem {
    pub path: PathBuf,
    pub file_name: String,
    pub meta: PackMeta,
}

/// Writes a pack to `dest`. Returns how many GIFs and bytes were written.
pub fn export(dest: &Path, items: &[ExportItem]) -> Result<(usize, u64), String> {
    let manifest = Manifest {
        format: FORMAT.into(),
        version: VERSION,
        gifs: items
            .iter()
            .map(|i| PackGif {
                file: i.file_name.clone(),
                meta: i.meta.clone(),
            })
            .collect(),
    };
    let json = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;

    // Write next to the destination first, so a failed export never leaves a broken pack.
    let tmp = dest.with_extension("gifpack.part");
    let result = (|| -> Result<u64, String> {
        let err = |e: zip::result::ZipError| e.to_string();
        let mut zip = ZipWriter::new(BufWriter::new(
            File::create(&tmp).map_err(|e| e.to_string())?,
        ));
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

        zip.start_file("pack.json", stored).map_err(err)?;
        zip.write_all(&json).map_err(|e| e.to_string())?;

        let mut bytes = 0;
        for item in items {
            let len = fs::metadata(&item.path).map_err(|e| e.to_string())?.len();
            let options = stored.large_file(len >= u32::MAX as u64);
            zip.start_file(format!("gifs/{}", item.file_name), options)
                .map_err(err)?;
            let mut file = BufReader::new(File::open(&item.path).map_err(|e| e.to_string())?);
            bytes += io::copy(&mut file, &mut zip).map_err(|e| e.to_string())?;
        }
        zip.finish()
            .map_err(err)?
            .flush()
            .map_err(|e| e.to_string())?;
        Ok(bytes)
    })();

    match result {
        Ok(bytes) => {
            fs::rename(&tmp, dest).map_err(|e| e.to_string())?;
            Ok((items.len(), bytes))
        }
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Extracts a pack's GIFs into `into` (a new, empty folder) and returns each
/// extracted file with its metadata.
pub fn extract(pack: &Path, into: &Path) -> Result<Vec<(PathBuf, PackMeta)>, String> {
    let err = |e: zip::result::ZipError| format!("not a valid GIF pack ({e})");
    let mut zip = ZipArchive::new(BufReader::new(File::open(pack).map_err(|e| e.to_string())?))
        .map_err(err)?;

    let manifest: Manifest = {
        let mut json = String::new();
        zip.by_name("pack.json")
            .map_err(err)?
            .read_to_string(&mut json)
            .map_err(|e| e.to_string())?;
        serde_json::from_str(&json).map_err(|e| format!("not a valid GIF pack ({e})"))?
    };
    if manifest.format != FORMAT {
        return Err("not a LocalGif GIF pack".into());
    }

    fs::create_dir_all(into).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for gif in manifest.gifs {
        // Only plain file names: a pack must never write outside `into`.
        let safe = Path::new(&gif.file)
            .file_name()
            .is_some_and(|n| n == gif.file.as_str());
        if !safe {
            continue;
        }
        let Ok(mut entry) = zip.by_name(&format!("gifs/{}", gif.file)) else {
            continue; // Listed but missing from the archive.
        };
        let dest = into.join(&gif.file);
        let mut file = BufWriter::new(File::create(&dest).map_err(|e| e.to_string())?);
        io::copy(&mut entry, &mut file).map_err(|e| e.to_string())?;
        file.flush().map_err(|e| e.to_string())?;
        out.push((dest, gif.meta));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_keeps_files_identical_and_metadata() {
        let root = std::env::temp_dir().join(format!("localgif-pack-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        // Two "GIFs" with arbitrary bytes: they must come back exactly the same.
        let a = root.join("cat dance.gif");
        let b = root.join("fire.gif");
        fs::write(&a, (0..=255u8).cycle().take(300_000).collect::<Vec<_>>()).unwrap();
        fs::write(&b, b"GIF89a tiny").unwrap();
        let items = vec![
            ExportItem {
                path: a.clone(),
                file_name: "cat dance.gif".into(),
                meta: PackMeta {
                    label: Some("Dancing cat".into()),
                    tags: vec!["cat".into(), "dance".into()],
                    favorite: true,
                    nsfw: false,
                    warning: true,
                },
            },
            ExportItem {
                path: b.clone(),
                file_name: "fire.gif".into(),
                meta: PackMeta {
                    nsfw: true,
                    ..Default::default()
                },
            },
        ];
        let pack = root.join("test.gifpack");
        let (count, bytes) = export(&pack, &items).unwrap();
        assert_eq!(count, 2);
        assert_eq!(bytes, 300_000 + 11);

        // Nothing is compressed.
        let mut zip = ZipArchive::new(File::open(&pack).unwrap()).unwrap();
        for i in 0..zip.len() {
            let f = zip.by_index(i).unwrap();
            assert_eq!(f.compression(), CompressionMethod::Stored);
        }

        let out = extract(&pack, &root.join("out")).unwrap();
        assert_eq!(out.len(), 2);
        let (path_a, meta_a) = &out[0];
        assert_eq!(fs::read(path_a).unwrap(), fs::read(&a).unwrap());
        assert_eq!(meta_a.label.as_deref(), Some("Dancing cat"));
        assert_eq!(meta_a.tags, vec!["cat", "dance"]);
        assert!(meta_a.favorite && meta_a.warning && !meta_a.nsfw);
        assert_eq!(fs::read(&out[1].0).unwrap(), b"GIF89a tiny");
        assert!(out[1].1.nsfw);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_paths_outside_the_import_folder() {
        let root = std::env::temp_dir().join(format!("localgif-pack-evil-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let pack = root.join("evil.gifpack");
        {
            let mut zip = ZipWriter::new(File::create(&pack).unwrap());
            let o = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            zip.start_file("pack.json", o).unwrap();
            zip.write_all(br#"{"format":"localgif-pack","version":1,"gifs":[{"file":"../escape.gif"}]}"#)
                .unwrap();
            zip.start_file("gifs/../escape.gif", o).unwrap();
            zip.write_all(b"x").unwrap();
            zip.finish().unwrap();
        }
        let out = extract(&pack, &root.join("out")).unwrap();
        assert!(out.is_empty());
        assert!(!root.join("escape.gif").exists());
        let _ = fs::remove_dir_all(&root);
    }
}
