// Copied pictures between devices: the picture on this computer's clipboard
// goes out as a PNG file (mdcore `Engine::send_clipboard_image`), and a
// picture another device copied is put on this clipboard when it arrives
// (`FileReceived { clipboard: true }`), besides being saved like any file.
//
// One clipboard handle is kept for the app's life: on Linux (X11) a copied
// picture is served by its owner, and goes away when the owner does.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

fn with_clipboard<T>(
    f: impl FnOnce(&mut arboard::Clipboard) -> Result<T, String>,
) -> Result<T, String> {
    static CB: OnceLock<Mutex<Option<arboard::Clipboard>>> = OnceLock::new();
    let lock = CB.get_or_init(|| Mutex::new(None));
    let mut g = lock.lock().map_err(|_| "clipboard busy".to_string())?;
    if g.is_none() {
        *g = Some(arboard::Clipboard::new().map_err(|e| format!("no clipboard: {e}"))?);
    }
    match g.as_mut() {
        Some(cb) => f(cb),
        None => Err("no clipboard".into()),
    }
}

/// The text on the clipboard, if any.
pub fn copied_text() -> Option<String> {
    with_clipboard(|cb| cb.get_text().map_err(|e| e.to_string())).ok()
}

/// The picture on the clipboard, saved as a PNG in a temporary folder.
pub fn copied_to_png() -> Result<PathBuf, String> {
    let img = with_clipboard(|cb| {
        cb.get_image()
            .map(|i| (i.width, i.height, i.bytes.into_owned()))
            .map_err(|_| "There's no picture on the clipboard. Copy one first.".to_string())
    })?;
    let (w, h, rgba) = img;
    let path = temp_png_path()?;
    let width = u32::try_from(w).map_err(|_| "picture too big")?;
    let height = u32::try_from(h).map_err(|_| "picture too big")?;
    write_png(&path, width, height, &rgba)?;
    Ok(path)
}

/// A new "Copied picture <time>.png" in the temporary folder pictures are
/// sent from (" 2", " 3"... added when one that second is there already).
fn temp_png_path() -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join("my-domain-clipboard");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut path = dir.join(format!("Copied picture {stamp}.png"));
    let mut n = 2;
    while path.exists() && n < 1000 {
        path = dir.join(format!("Copied picture {stamp} {n}.png"));
        n += 1;
    }
    Ok(path)
}

/// A copy of the PNG at `src` in the temporary folder, to send from (so the
/// kept one may go while it's on its way).
pub fn temp_copy(src: &Path) -> Result<PathBuf, String> {
    let path = temp_png_path()?;
    std::fs::copy(src, &path).map_err(|e| format!("can't read the picture: {e}"))?;
    Ok(path)
}

/// Write 8-bit RGBA pixels as a PNG at `path`.
pub fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut w| w.write_image_data(rgba))
        .map_err(|e| format!("can't save the picture: {e}"))
}

/// The picture (`width` x `height`, 8-bit RGBA) shrunk so its longer side is
/// at most `max` pixels (never enlarged), each pixel the average of the ones
/// it covers. Its width, height and pixels.
pub fn thumbnail(width: u32, height: u32, rgba: &[u8], max: u32) -> (u32, u32, Vec<u8>) {
    let long = width.max(height);
    if long <= max || max == 0 {
        return (width, height, rgba.to_vec());
    }
    let scale = |v: u32| {
        let s = (u64::from(v) * u64::from(max) + u64::from(long) / 2) / u64::from(long);
        u32::try_from(s.max(1)).unwrap_or(max)
    };
    let (tw, th) = (scale(width) as usize, scale(height) as usize);
    let (w, h) = (width as usize, height as usize);
    let mut out = Vec::with_capacity(tw * th * 4);
    for ty in 0..th {
        let y0 = ty * h / th;
        let y1 = ((ty + 1) * h / th).max(y0 + 1);
        for tx in 0..tw {
            let x0 = tx * w / tw;
            let x1 = ((tx + 1) * w / tw).max(x0 + 1);
            let mut sum = [0u64; 4];
            for y in y0..y1 {
                let row = rgba.get((y * w + x0) * 4..(y * w + x1) * 4).unwrap_or(&[]);
                for p in row.as_chunks::<4>().0 {
                    for (s, &c) in sum.iter_mut().zip(p) {
                        *s += u64::from(c);
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u64;
            out.extend(sum.iter().map(|s| u8::try_from(s / n).unwrap_or(u8::MAX)));
        }
    }
    (tw as u32, th as u32, out)
}

/// Put the PNG at `path` on the clipboard.
pub fn put_on_clipboard(path: &Path) -> Result<(), String> {
    let (w, h, rgba) = read_png(path)?;
    with_clipboard(|cb| {
        cb.set_image(arboard::ImageData {
            width: w,
            height: h,
            bytes: rgba.into(),
        })
        .map_err(|e| format!("can't copy the picture: {e}"))
    })
}

/// A PNG's size and pixels as 8-bit RGBA.
pub fn read_png(path: &Path) -> Result<(usize, usize, Vec<u8>), String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut dec = png::Decoder::new(std::io::BufReader::new(file));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = dec.read_info().map_err(|e| format!("not a PNG: {e}"))?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("not a PNG: {e}"))?;
    let px = &buf[..info.buffer_size()];
    let rgba = to_rgba(px, info.color_type).ok_or("unsupported PNG colours")?;
    Ok((info.width as usize, info.height as usize, rgba))
}

/// 8-bit pixels of `color` as RGBA.
fn to_rgba(px: &[u8], color: png::ColorType) -> Option<Vec<u8>> {
    Some(match color {
        png::ColorType::Rgba => px.to_vec(),
        png::ColorType::Rgb => px.chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::Grayscale => px.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => px
            .chunks(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Indexed => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::{thumbnail, to_rgba};

    #[test]
    fn thumbnails_fit_in_the_box() {
        let px = |w: u32, h: u32| vec![200u8; (w * h * 4) as usize];
        let (w, h, out) = thumbnail(1024, 512, &px(1024, 512), 256);
        assert_eq!((w, h), (256, 128));
        assert_eq!(out.len(), 256 * 128 * 4);
        assert!(out.iter().all(|&c| c == 200));
        let (w, h, _) = thumbnail(300, 1000, &px(300, 1000), 256);
        assert_eq!((w, h), (77, 256));
        // A thin one keeps at least a pixel across.
        let (w, h, _) = thumbnail(2000, 1, &px(2000, 1), 256);
        assert_eq!((w, h), (256, 1));
        // A small one stays as it is.
        let (w, h, out) = thumbnail(10, 20, &px(10, 20), 256);
        assert_eq!((w, h, out.len()), (10, 20, 800));
    }

    #[test]
    fn thumbnails_average_what_they_cover() {
        // 2x1, black and white: one grey pixel.
        let (w, h, out) = thumbnail(2, 1, &[0, 0, 0, 255, 255, 255, 255, 255], 1);
        assert_eq!((w, h), (1, 1));
        assert_eq!(out, [127, 127, 127, 255]);
    }

    #[test]
    fn png_pixels_become_rgba() {
        assert_eq!(
            to_rgba(&[1, 2, 3], png::ColorType::Rgb),
            Some(vec![1, 2, 3, 255])
        );
        assert_eq!(
            to_rgba(&[9, 7], png::ColorType::GrayscaleAlpha),
            Some(vec![9, 9, 9, 7])
        );
        assert_eq!(
            to_rgba(&[5], png::ColorType::Grayscale),
            Some(vec![5, 5, 5, 255])
        );
        assert_eq!(to_rgba(&[0], png::ColorType::Indexed), None);
    }
}
