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

/// The picture on the clipboard, saved as a PNG in a temporary folder.
pub fn copied_to_png() -> Result<PathBuf, String> {
    let img = with_clipboard(|cb| {
        cb.get_image()
            .map(|i| (i.width, i.height, i.bytes.into_owned()))
            .map_err(|_| "There's no picture on the clipboard. Copy one first.".to_string())
    })?;
    let (w, h, rgba) = img;
    let dir = std::env::temp_dir().join("my-domain-clipboard");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("Copied picture {stamp}.png"));
    let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
    let width = u32::try_from(w).map_err(|_| "picture too big")?;
    let height = u32::try_from(h).map_err(|_| "picture too big")?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut w| w.write_image_data(&rgba))
        .map_err(|e| format!("can't save the picture: {e}"))?;
    Ok(path)
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
fn read_png(path: &Path) -> Result<(usize, usize, Vec<u8>), String> {
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
    use super::to_rgba;

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
