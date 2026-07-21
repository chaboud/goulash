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
    /// Q;<base64 text> — a `#` aside intercepted at accept-line
    Ask(String),
    /// S;<base64 buffer> — line editor requests a suggestion pull; the
    /// current buffer contents let goulash cycle through the list
    Pull(String),
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

    /// Returns clean bytes and marks as ordered segments, so callers can
    /// attribute output to the correct command block even when the
    /// B/output/D sequence of a fast command arrives in one read.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<Seg> {
        let mut data = std::mem::take(&mut self.pending);
        data.extend_from_slice(chunk);

        let mut segs: Vec<Seg> = Vec::new();
        let mut run: Vec<u8> = Vec::new();
        let mut i = 0;
        while i < data.len() {
            if data[i] != 0x1b {
                let next = data[i..]
                    .iter()
                    .position(|&b| b == 0x1b)
                    .map(|p| i + p)
                    .unwrap_or(data.len());
                run.extend_from_slice(&data[i..next]);
                i = next;
                continue;
            }
            let rest = &data[i..];
            let m = MARKER.len().min(rest.len());
            if rest[..m] != MARKER[..m] {
                run.push(0x1b);
                i += 1;
                continue;
            }
            if m < MARKER.len() {
                // Partial marker at the end of the chunk; wait for more.
                self.pending = rest.to_vec();
                flush_run(&mut segs, &mut run);
                return segs;
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
                        flush_run(&mut segs, &mut run);
                        segs.push(Seg::Mark(mark));
                    }
                    i = e + tlen;
                }
                None => {
                    if data.len() - i <= MAX_PENDING {
                        self.pending = data[i..].to_vec();
                    } else {
                        // Unterminated and oversized: give up, forward raw.
                        run.extend_from_slice(&data[i..]);
                    }
                    flush_run(&mut segs, &mut run);
                    return segs;
                }
            }
        }
        flush_run(&mut segs, &mut run);
        segs
    }
}

/// One ordered piece of a filtered chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seg {
    Bytes(Vec<u8>),
    Mark(Mark),
}

fn flush_run(segs: &mut Vec<Seg>, run: &mut Vec<u8>) {
    if !run.is_empty() {
        segs.push(Seg::Bytes(std::mem::take(run)));
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
        b"Q" => Some(Mark::Ask(b64(payload)?)),
        b"S" => Some(Mark::Pull(if payload.is_empty() {
            String::new()
        } else {
            b64(payload)?
        })),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(s: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(s)
    }

    fn split(segs: Vec<Seg>) -> (Vec<u8>, Vec<Mark>) {
        let mut bytes = Vec::new();
        let mut marks = Vec::new();
        for s in segs {
            match s {
                Seg::Bytes(b) => bytes.extend(b),
                Seg::Mark(m) => marks.push(m),
            }
        }
        (bytes, marks)
    }

    #[test]
    fn passthrough_and_extract() {
        let mut f = OscFilter::new();
        let input = format!(
            "hello\x1b]7770;A\x07world\x1b]7770;B;{}\x07!",
            enc("ls -la")
        );
        let (clean, marks) = split(f.feed(input.as_bytes()));
        assert_eq!(clean, b"helloworld!");
        assert_eq!(
            marks,
            vec![Mark::Prompt, Mark::CmdStart("ls -la".to_string())]
        );
    }

    #[test]
    fn ordering_preserved() {
        let mut f = OscFilter::new();
        let input = format!(
            "\x1b]7770;B;{}\x07output-text\x1b]7770;D;1\x07",
            enc("false")
        );
        let segs = f.feed(input.as_bytes());
        assert_eq!(
            segs,
            vec![
                Seg::Mark(Mark::CmdStart("false".to_string())),
                Seg::Bytes(b"output-text".to_vec()),
                Seg::Mark(Mark::CmdEnd(1)),
            ]
        );
    }

    #[test]
    fn split_across_chunks() {
        let full = "a\x1b]7770;D;3\x07b";
        let bytes = full.as_bytes();
        for cut in 1..bytes.len() {
            let mut f = OscFilter::new();
            let (mut clean, mut marks) = split(f.feed(&bytes[..cut]));
            let (c2, m2) = split(f.feed(&bytes[cut..]));
            clean.extend(c2);
            marks.extend(m2);
            assert_eq!(clean, b"ab", "cut at {cut}");
            assert_eq!(marks, vec![Mark::CmdEnd(3)], "cut at {cut}");
        }
    }

    #[test]
    fn st_terminator_and_unknown_osc() {
        let mut f = OscFilter::new();
        let (clean, marks) = split(f.feed(b"x\x1b]7770;A\x1b\\y\x1b]0;title\x07z"));
        assert_eq!(clean, b"xy\x1b]0;title\x07z" as &[u8]);
        assert_eq!(marks, vec![Mark::Prompt]);
    }

    #[test]
    fn cwd_mark() {
        let mut f = OscFilter::new();
        let input = format!("\x1b]7770;P;{}\x07", enc("/home/user/proj"));
        let (clean, marks) = split(f.feed(input.as_bytes()));
        assert!(clean.is_empty());
        assert_eq!(marks, vec![Mark::Cwd("/home/user/proj".to_string())]);
    }
}
