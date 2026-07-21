use base64::Engine as _;

/// Private OSC channel from the shell integration scripts (shell/goulash.*):
/// `ESC ] 7770 ; <tag> [; payload] (BEL | ESC \)`
///
/// Marks are stripped from the stream before it reaches the real terminal
/// and surfaced as structured events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mark {
    /// A — prompt displayed (precmd)
    Prompt,
    /// B;<base64 cmd> — command about to execute (preexec)
    CmdStart(String),
    /// D;<exit code> — command finished
    CmdEnd(i32),
    /// P;<base64 cwd> — working directory report
    Cwd(String),
}

const MARKER: &[u8] = b"\x1b]7770;";
const MAX_PENDING: usize = 8192;

/// Streaming filter: extracts goulash OSC marks from PTY output, tolerating
/// sequences split across read boundaries via a bounded carry-over buffer.
pub struct OscFilter {
    pending: Vec<u8>,
}

impl OscFilter {
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
        }
    }

    /// Returns (clean bytes to forward, marks found).
    pub fn feed(&mut self, chunk: &[u8]) -> (Vec<u8>, Vec<Mark>) {
        let mut data = std::mem::take(&mut self.pending);
        data.extend_from_slice(chunk);

        let mut out = Vec::with_capacity(data.len());
        let mut marks = Vec::new();
        let mut i = 0;
        while i < data.len() {
            if data[i] != 0x1b {
                let next = data[i..]
                    .iter()
                    .position(|&b| b == 0x1b)
                    .map(|p| i + p)
                    .unwrap_or(data.len());
                out.extend_from_slice(&data[i..next]);
                i = next;
                continue;
            }
            let rest = &data[i..];
            let m = MARKER.len().min(rest.len());
            if rest[..m] != MARKER[..m] {
                out.push(0x1b);
                i += 1;
                continue;
            }
            if m < MARKER.len() {
                // Partial marker at the end of the chunk; wait for more.
                self.pending = rest.to_vec();
                return (out, marks);
            }
            // Full marker: look for BEL or ST terminator.
            let body_start = i + MARKER.len();
            let mut j = body_start;
            let mut end = None;
            while j < data.len() {
                match data[j] {
                    0x07 => {
                        end = Some((j, 1));
                        break;
                    }
                    0x1b if j + 1 < data.len() && data[j + 1] == b'\\' => {
                        end = Some((j, 2));
                        break;
                    }
                    0x1b if j + 1 >= data.len() => break, // possibly split ST
                    _ => j += 1,
                }
            }
            match end {
                Some((e, tlen)) => {
                    if let Some(mark) = parse_mark(&data[body_start..e]) {
                        marks.push(mark);
                    }
                    i = e + tlen;
                }
                None => {
                    if data.len() - i <= MAX_PENDING {
                        self.pending = data[i..].to_vec();
                    } else {
                        // Unterminated and oversized: give up, forward raw.
                        out.extend_from_slice(&data[i..]);
                    }
                    return (out, marks);
                }
            }
        }
        (out, marks)
    }
}

fn b64(s: &[u8]) -> Option<String> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(s).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn parse_mark(body: &[u8]) -> Option<Mark> {
    let (tag, payload) = match body.iter().position(|&b| b == b';') {
        Some(p) => (&body[..p], &body[p + 1..]),
        None => (body, &body[body.len()..]),
    };
    match tag {
        b"A" => Some(Mark::Prompt),
        b"B" => Some(Mark::CmdStart(b64(payload)?)),
        b"D" => std::str::from_utf8(payload)
            .ok()?
            .trim()
            .parse::<i32>()
            .ok()
            .map(Mark::CmdEnd),
        b"P" => Some(Mark::Cwd(b64(payload)?)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    fn enc(s: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(s)
    }

    #[test]
    fn passthrough_and_extract() {
        let mut f = OscFilter::new();
        let input = format!(
            "hello\x1b]7770;A\x07world\x1b]7770;B;{}\x07!",
            enc("ls -la")
        );
        let (clean, marks) = f.feed(input.as_bytes());
        assert_eq!(clean, b"helloworld!");
        assert_eq!(
            marks,
            vec![Mark::Prompt, Mark::CmdStart("ls -la".to_string())]
        );
    }

    #[test]
    fn split_across_chunks() {
        let mut f = OscFilter::new();
        let full = format!("a\x1b]7770;D;3\x07b");
        let bytes = full.as_bytes();
        for cut in 1..bytes.len() {
            let mut f2 = OscFilter::new();
            let (mut clean, mut marks) = f2.feed(&bytes[..cut]);
            let (c2, m2) = f2.feed(&bytes[cut..]);
            clean.extend_from_slice(&c2);
            marks.extend(m2);
            assert_eq!(clean, b"ab", "cut at {cut}");
            assert_eq!(marks, vec![Mark::CmdEnd(3)], "cut at {cut}");
        }
        let _ = f;
    }

    #[test]
    fn st_terminator_and_unknown_osc() {
        let mut f = OscFilter::new();
        let (clean, marks) = f.feed(b"x\x1b]7770;A\x1b\\y\x1b]0;title\x07z");
        // Goulash mark extracted; unknown OSC 0 forwarded verbatim.
        assert_eq!(clean, b"xy\x1b]0;title\x07z" as &[u8]);
        assert_eq!(marks, vec![Mark::Prompt]);
    }

    #[test]
    fn cwd_mark() {
        let mut f = OscFilter::new();
        let input = format!("\x1b]7770;P;{}\x07", enc("/home/user/proj"));
        let (clean, marks) = f.feed(input.as_bytes());
        assert!(clean.is_empty());
        assert_eq!(marks, vec![Mark::Cwd("/home/user/proj".to_string())]);
    }
}
