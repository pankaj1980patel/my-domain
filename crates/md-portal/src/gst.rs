//! The screen cast's PipeWire stream read by GStreamer: the command line
//! (arguments as a vector, never through a shell) and the JPEGs it writes,
//! split from its multipart output.

/// The GStreamer launcher.
pub const LAUNCH: &str = "gst-launch-1.0";
/// GStreamer's element inspector (to see that the elements are there).
pub const INSPECT: &str = "gst-inspect-1.0";
/// The elements the pipeline needs beyond GStreamer's base plugins.
pub const ELEMENTS: [&str; 2] = ["pipewiresrc", "jpegenc"];
/// Between two pictures in the output.
pub const BOUNDARY: &str = "mdframe";

/// `gst-launch-1.0` arguments: the PipeWire stream `node` (opened through
/// the portal's remote, inherited as `fd`), `fps` pictures a second,
/// `width` pixels wide (the height follows), JPEG `quality` (1-100), out
/// on stdout between `BOUNDARY` lines with their length.
pub fn pipeline(fd: i32, node: u32, width: u32, fps: u32, quality: u8) -> Vec<String> {
    let s = |v: &str| v.to_string();
    vec![
        s("-q"),
        s("pipewiresrc"),
        format!("fd={fd}"),
        format!("path={node}"),
        s("do-timestamp=true"),
        // An idle screen sends nothing: the last picture again each second.
        s("keepalive-time=1000"),
        s("!"),
        s("videorate"),
        s("!"),
        format!("video/x-raw,framerate={}/1", fps.max(1)),
        s("!"),
        s("videoconvert"),
        s("!"),
        s("videoscale"),
        s("!"),
        format!("video/x-raw,width={},pixel-aspect-ratio=1/1", width.max(16)),
        s("!"),
        s("jpegenc"),
        format!("quality={}", quality.clamp(1, 100)),
        s("!"),
        s("multipartmux"),
        format!("boundary={BOUNDARY}"),
        s("!"),
        s("fdsink"),
        s("fd=1"),
    ]
}

/// The pictures in the multipart output, as it comes in pieces.
#[derive(Default)]
pub struct Frames {
    buf: Vec<u8>,
}

/// The most kept while looking for a picture's end (a stuck stream
/// mustn't grow without bound).
const MAX_BUF: usize = 16 * 1024 * 1024;

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

impl Frames {
    /// Add what was read.
    pub fn push(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
        if self.buf.len() > MAX_BUF {
            self.buf.clear();
        }
    }

    /// The next whole picture, if one is in.
    pub fn next_frame(&mut self) -> Option<Vec<u8>> {
        let mark = format!("--{BOUNDARY}");
        let start = find(&self.buf, mark.as_bytes())?;
        let head_end = find(&self.buf[start..], b"\r\n\r\n")? + start + 4;
        let head = String::from_utf8_lossy(&self.buf[start..head_end]).to_ascii_lowercase();
        let len = head
            .lines()
            .find_map(|l| l.strip_prefix("content-length:"))
            .and_then(|n| n.trim().parse::<usize>().ok());
        let (body_end, next) = match len {
            Some(n) => {
                if self.buf.len() < head_end + n {
                    return None;
                }
                (head_end + n, head_end + n)
            }
            // No length: up to the next boundary.
            None => {
                let at = find(&self.buf[head_end..], mark.as_bytes())? + head_end;
                let mut end = at;
                if self.buf[..end].ends_with(b"\r\n") {
                    end -= 2;
                }
                (end, at)
            }
        };
        let frame = self.buf[head_end..body_end].to_vec();
        self.buf.drain(..next);
        Some(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pipeline_reads_the_portal_stream_into_jpegs() {
        let a = pipeline(7, 42, 1280, 2, 50);
        assert_eq!(a[0], "-q");
        assert_eq!(a[1], "pipewiresrc");
        assert!(a.contains(&"fd=7".to_string()));
        assert!(a.contains(&"path=42".to_string()));
        assert!(a.contains(&"video/x-raw,framerate=2/1".to_string()));
        assert!(a.contains(&"video/x-raw,width=1280,pixel-aspect-ratio=1/1".to_string()));
        assert!(a.contains(&"quality=50".to_string()));
        assert_eq!(a[a.len() - 2..], ["fdsink".to_string(), "fd=1".to_string()]);
        // Each argument is one word: nothing for a shell to split.
        assert!(a.iter().all(|x| !x.contains(' ')));
        let silly = pipeline(3, 1, 0, 0, 200);
        assert!(silly.contains(&"quality=100".to_string()));
        assert!(silly.contains(&"video/x-raw,framerate=1/1".to_string()));
    }

    fn part(body: &[u8], with_len: bool) -> Vec<u8> {
        let mut v = format!("\r\n--{BOUNDARY}\r\nContent-Type: image/jpeg\r\n").into_bytes();
        if with_len {
            v.extend_from_slice(format!("Content-Length: {}\r\n", body.len()).as_bytes());
        }
        v.extend_from_slice(b"\r\n");
        v.extend_from_slice(body);
        v
    }

    #[test]
    fn pictures_come_out_whole_however_the_output_is_cut() {
        let one = [0xffu8, 0xd8, 1, 2, 3, b'-', b'-', 0xff, 0xd9];
        let two = [0xffu8, 0xd8, 9, 0xff, 0xd9];
        let mut all = part(&one, true);
        all.extend(part(&two, true));
        let mut f = Frames::default();
        let mut got = Vec::new();
        for chunk in all.chunks(3) {
            f.push(chunk);
            while let Some(p) = f.next_frame() {
                got.push(p);
            }
        }
        assert_eq!(got, vec![one.to_vec(), two.to_vec()]);
        assert_eq!(f.next_frame(), None);
    }

    #[test]
    fn without_a_length_a_picture_ends_at_the_next_boundary() {
        let mut f = Frames::default();
        f.push(&part(b"abc", false));
        assert_eq!(f.next_frame(), None);
        f.push(&part(b"def", false));
        assert_eq!(f.next_frame(), Some(b"abc".to_vec()));
    }
}
