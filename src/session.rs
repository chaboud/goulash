use crate::config::Config;
use crate::engine::{self, Engine};
use crate::memory::MemoryStore;
use crate::osc::{Mark, OscFilter, Seg};
use crate::pty;
use crate::record::Recorder;
use crate::sense::{self, HookPhase, Sensor, State};
use crate::state::StateFile;
use crate::status;
use crate::term::{self, RawGuard, Size};
use crate::vendor::{self, RulesVendor};
use nix::poll::{PollFd, PollFlags, PollTimeout};
use nix::sys::signal::{self, SaFlags, SigAction, SigHandler, SigSet, Signal};
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, RawFd};
use std::os::unix::process::ExitStatusExt;
use std::sync::atomic::{AtomicI32, Ordering};

const STDIN: RawFd = 0;
const STDOUT: RawFd = 1;

/// Write end of the SIGWINCH self-pipe; -1 until installed.
static WINCH_PIPE_WR: AtomicI32 = AtomicI32::new(-1);

extern "C" fn on_sigwinch(_: libc::c_int) {
    let fd = WINCH_PIPE_WR.load(Ordering::Relaxed);
    if fd >= 0 {
        // SAFETY: write(2) is async-signal-safe; best-effort wakeup.
        unsafe { libc::write(fd, b"w".as_ptr().cast(), 1) };
    }
}

fn write_all(fd: RawFd, mut buf: &[u8]) -> io::Result<()> {
    while !buf.is_empty() {
        // SAFETY: fd is a valid open descriptor for the duration of the session.
        let n = unsafe { libc::write(fd, buf.as_ptr().cast(), buf.len()) };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        buf = &buf[n as usize..];
    }
    Ok(())
}

fn read_some(fd: RawFd, buf: &mut [u8]) -> io::Result<usize> {
    loop {
        // SAFETY: fd valid, buf sized by caller.
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        return Ok(n as usize);
    }
}

/// Ask the real terminal where the cursor is (DSR). Bytes that arrive that
/// are not part of the report are returned as `typed_ahead` for forwarding
/// to the shell. Falls back to bottom-of-screen on timeout so the reserved
/// rows always end up clear of existing content.
fn query_cursor_row(real: Size) -> (u16, u16, Vec<u8>) {
    let _ = write_all(STDOUT, b"\x1b[6n");
    let mut acc: Vec<u8> = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
    loop {
        let remain = deadline.saturating_duration_since(std::time::Instant::now());
        if remain.is_zero() {
            break;
        }
        let stdin_fd = unsafe { BorrowedFd::borrow_raw(STDIN) };
        let mut fds = [PollFd::new(stdin_fd, PollFlags::POLLIN)];
        let timeout = PollTimeout::try_from(remain.as_millis().max(1) as i32).unwrap();
        match nix::poll::poll(&mut fds, timeout) {
            Ok(0) => break,
            Ok(_) => {
                let mut byte = [0u8; 64];
                match read_some(STDIN, &mut byte) {
                    Ok(0) => break,
                    Ok(n) => {
                        acc.extend_from_slice(&byte[..n]);
                        if acc.contains(&b'R') {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            Err(_) => break,
        }
    }
    // Find the last ESC [ row ; col R in the accumulated bytes.
    if let Some(start) = acc.windows(2).rposition(|w| w == b"\x1b[")
        && let Some(end_rel) = acc[start..].iter().position(|&b| b == b'R')
    {
        let body = &acc[start + 2..start + end_rel];
        let text: String = body.iter().map(|&b| b as char).collect();
        let mut parts = text.split(';');
        if let (Some(r), Some(c)) = (parts.next(), parts.next())
            && let (Ok(row), Ok(col)) = (r.parse::<u16>(), c.parse::<u16>())
        {
            let mut typed_ahead = acc[..start].to_vec();
            typed_ahead.extend_from_slice(&acc[start + end_rel + 1..]);
            return (row.min(real.rows), col.min(real.cols), typed_ahead);
        }
    }
    (real.rows, 1, acc)
}

/// Sequences in the child's output that can blow away our scroll region or
/// status row: DECSTBM reset, RIS, DECSTR soft reset, full clears — and
/// erase-below (ESC[J / ESC[0J), which line editors emit on every refresh
/// and which erases to the end of the *real* screen, straight through the
/// reserved rows (ED is not bounded by scroll regions). The caller repaints
/// in the same write batch as the trigger, so the wiped bar never renders.
/// Returns the end index of the earliest trigger in `chunk`, so the caller
/// can re-pin the scroll region at exactly that boundary — before any
/// scrolling output that follows it in the same chunk can drag the status
/// row up into the inner region.
fn find_trigger_end(chunk: &[u8]) -> Option<usize> {
    const TRIGGERS: [&[u8]; 7] = [
        b"\x1b[r", b"\x1bc", b"[!p", b"[2J", b"[3J", b"\x1b[J", b"\x1b[0J",
    ];
    TRIGGERS
        .iter()
        .filter_map(|t| {
            chunk
                .windows(t.len())
                .position(|w| w == *t)
                .map(|i| i + t.len())
        })
        .min()
}

struct Layout {
    real: Size,
    reserved: u16,
}

impl Layout {
    fn inner(&self) -> Size {
        Size {
            rows: (self.real.rows.saturating_sub(self.reserved)).max(1),
            cols: self.real.cols,
        }
    }
    fn status_row(&self) -> u16 {
        // 1-based real row of the (first) reserved row.
        self.inner().rows + 1
    }
}

/// The heckle band's content: the asked question (collapsed when the
/// answer wasn't user-prompted) and the answer/explanation text.
struct Band {
    question: Option<String>,
    text: String,
}

/// One vended (suggestion, chat) turn in the slot history — the
/// single-slot scrollable stack Down cycles through
/// (wiki: interaction/down-arrow-protocol.md). Commands are the anchor;
/// the paired chat text rides along for the band.
#[derive(Clone)]
struct SugTurn {
    id: u64,
    cmd: String,
    text: String,
}

const SUG_HIST_CAP: usize = 50;

/// The shared menu primitive (wiki: interaction/settings-and-nav.md):
/// modal, type-to-filter (no per-item hotkeys), the list scrolling under
/// a fixed cursor inside the fixed goulash area — no winsize change.
/// Only ever opened by the user, by name (bare `#/model`); Esc and
/// Ctrl-C always close it.
struct Menu {
    title: String,
    items: Vec<String>,
    filter: String,
    cursor: usize, // index into the filtered view
    loaded: bool,
}

impl Menu {
    fn open(title: &str) -> Menu {
        Menu {
            title: title.to_string(),
            items: Vec::new(),
            filter: String::new(),
            cursor: 0,
            loaded: false,
        }
    }

    fn filtered(&self) -> Vec<&str> {
        let f = self.filter.to_lowercase();
        self.items
            .iter()
            .filter(|i| f.is_empty() || i.to_lowercase().contains(&f))
            .map(|s| s.as_str())
            .collect()
    }

    fn clamp(&mut self) {
        let n = self.filtered().len();
        self.cursor = self.cursor.min(n.saturating_sub(1));
    }
}

/// `##` chat focus (wiki: interaction/chat-mode.md): goulash owns the
/// keyboard for a multi-turn conversation — no `#` retyping. Kept pure:
/// commands only ever exit through the real shell line (Up hands the
/// newest suggestion over and focus flips back); there is no
/// act-observe loop here.
struct Chat {
    /// Transcript lines: "# question" / "goulash: answer".
    lines: Vec<String>,
    input: String,
    /// Streaming partial for the in-flight ask, shown as a live line.
    stream: Option<String>,
    /// Slot-stack selection: the same axis as at the prompt. None =
    /// neutral (typing); Down dives older, Up walks back to neutral,
    /// Enter on a selection hands that command to the shell.
    sel: Option<usize>,
}

/// One parsed keypress for goulash-owned surfaces (menus, chat line).
enum Key {
    Char(char),
    Enter,
    Backspace,
    KillLine,
    Up,
    Down,
    Esc,
    CtrlC,
}

/// Parse a raw stdin chunk into keys. Arrow sequences arrive whole in a
/// chunk; a lone ESC byte is treated as Esc (good enough at human typing
/// speeds — the classic terminal ambiguity).
fn parse_keys(chunk: &[u8]) -> Vec<Key> {
    let mut keys = Vec::new();
    let mut i = 0;
    while i < chunk.len() {
        match chunk[i] {
            0x1b => {
                match chunk.get(i + 1) {
                    Some(&b'[') => {
                        // Consume the whole CSI so parameterized
                        // sequences (e.g. 1;3B) never leak into a filter.
                        let mut j = i + 2;
                        while j < chunk.len() && !(0x40..0x7f).contains(&chunk[j]) {
                            j += 1;
                        }
                        if j < chunk.len() {
                            let plain = j == i + 2;
                            match chunk[j] {
                                b'A' if plain => keys.push(Key::Up),
                                b'B' if plain => keys.push(Key::Down),
                                _ => {}
                            }
                            i = j + 1;
                        } else {
                            i = chunk.len(); // truncated CSI: drop
                        }
                    }
                    // SS3 form (ESC O A/B): what real terminals send in
                    // application-cursor mode — which zsh's zle enables,
                    // so THIS is the arrow encoding live sessions use.
                    Some(&b'O') => {
                        if let Some(&f) = chunk.get(i + 2) {
                            match f {
                                b'A' => keys.push(Key::Up),
                                b'B' => keys.push(Key::Down),
                                _ => {}
                            }
                            i += 3;
                        } else {
                            i = chunk.len(); // truncated SS3: drop
                        }
                    }
                    _ => {
                        keys.push(Key::Esc);
                        i += 1;
                    }
                }
            }
            0x03 => {
                keys.push(Key::CtrlC);
                i += 1;
            }
            b'\r' | b'\n' => {
                keys.push(Key::Enter);
                i += 1;
            }
            0x7f | 0x08 => {
                keys.push(Key::Backspace);
                i += 1;
            }
            0x15 => {
                keys.push(Key::KillLine);
                i += 1;
            }
            c if (0x20..0x7f).contains(&c) => {
                keys.push(Key::Char(c as char));
                i += 1;
            }
            _ => i += 1,
        }
    }
    keys
}

fn hist_push(hist: &mut Vec<SugTurn>, turn: SugTurn) {
    if hist.first().map(|t| t.cmd == turn.cmd).unwrap_or(false) {
        return; // adjacent dedup: re-vending the same fix isn't a new turn
    }
    hist.insert(0, turn);
    hist.truncate(SUG_HIST_CAP);
}

/// Whitespace-collapse then hard-wrap into at most `max_rows` rows.
fn wrap_chars(s: &str, width: usize, max_rows: usize) -> Vec<String> {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut rows = Vec::new();
    let mut cur = String::new();
    for ch in flat.chars() {
        cur.push(ch);
        if cur.chars().count() >= width.max(8) {
            rows.push(std::mem::take(&mut cur));
            if rows.len() >= max_rows {
                return rows;
            }
        }
    }
    if !cur.trim().is_empty() {
        rows.push(cur);
    }
    rows
}

/// Compose the goulash area, top to bottom: rule row with the pullable
/// suggestion (what Down reaches first) or a notice cutting in; the
/// question and answer excerpt on plain terminal background; and the
/// chrome chip bottom-right. FIXED height: rows are blank when idle so
/// the terminal never resizes mid-session.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn compose_rows(
    cfg: &Config,
    layout: &Layout,
    shell_name: &str,
    st: &State,
    hook: Option<HookPhase>,
    suggestions: &[(u64, String, String)],
    notice: &Option<String>,
    band: &Option<Band>,
    browse: Option<usize>,
    sug_hist: &[SugTurn],
    menu: &Option<Menu>,
    engine_model: &Option<String>,
    chat: &Option<Chat>,
) -> Vec<String> {
    let cols = layout.real.cols as usize;
    let reserved_now = cfg.reserved_rows();
    let inner_now = layout.real.rows.saturating_sub(reserved_now).max(1);
    if cfg.status.band
        && menu.is_none()
        && let Some(c) = chat
    {
        // Chat has focus: the area grows a few rows (a user-initiated
        // resize — they toggled it) and shows the transcript tail plus
        // an input line. compose returning more rows IS the resize; the
        // winsize machinery does the rest.
        let extra = 4.min(layout.real.rows.saturating_sub(reserved_now + 8));
        // One transcript row cedes to the slot row at the BOTTOM of the
        // panel, so Down is spatially honest all the way: input above,
        // suggestion below, shell line beyond (Enter sends it up there).
        let n_chat = (cfg.status.band_rows.clamp(1, 4) + extra).saturating_sub(1) as usize;
        let reserved = reserved_now + extra;
        let inner = layout.real.rows.saturating_sub(reserved).max(1);
        let mut rows = Vec::new();
        let tip = " \u{23ce} send \u{b7} \u{2193} command \u{b7} ## or esc back ";
        rows.push(status::rule_row(Some(" ## chat "), true, Some(tip), cols));
        let mut tail: Vec<&str> = c.lines.iter().map(|s| s.as_str()).collect();
        let stream_line = c.stream.as_ref().map(|s| format!("goulash: {s} \u{2026}"));
        if let Some(sl) = stream_line.as_deref() {
            tail.push(sl);
        }
        let skip = tail.len().saturating_sub(n_chat);
        for row in 0..n_chat {
            let line = tail.get(skip + row).copied().unwrap_or("");
            let sgr = if line.starts_with('#') {
                status::QUERY_SGR
            } else {
                status::TEXT_SGR
            };
            rows.push(status::pad_row(&format!(" {line}"), cols, sgr));
        }
        rows.push(status::pad_row(
            &format!(" ## {}\u{258f}", c.input),
            cols,
            status::TEXT_SGR,
        ));
        // The slot row: selected = the orange band, its up-arrow saying
        // "Enter inserts this up into your shell line"; unselected = a
        // dim hint that Down reaches it.
        let slot_row = match c.sel.and_then(|i| sug_hist.get(i).map(|t| (i, t))) {
            Some((i, t)) => status::pad_row(
                &format!(
                    " \u{2191} {} \u{b7} {}/{}{}",
                    t.cmd,
                    i + 1,
                    sug_hist.len(),
                    if i + 1 < sug_hist.len() {
                        " \u{2193}"
                    } else {
                        ""
                    }
                ),
                cols,
                status::SUGGEST_SGR,
            ),
            None => match sug_hist.first() {
                Some(t) => status::pad_row(
                    &format!(" \u{2193} suggestion: {}", t.cmd),
                    cols,
                    status::QUERY_SGR,
                ),
                None => status::pad_row("", cols, status::TEXT_SGR),
            },
        };
        rows.push(slot_row);
        rows.push(status::chrome_row(
            layout.real,
            inner,
            reserved,
            shell_name,
            sense::label(st, hook),
        ));
        return rows;
    }
    if cfg.status.band
        && let Some(m) = menu
    {
        // Modal menu: the list scrolls under a fixed cursor inside the
        // fixed area (TV-menu style — no winsize change, nothing
        // reflows). Rows: rule (title/filter + keymap), items, chrome.
        let n_items = (cfg.status.band_rows.clamp(1, 4) + 1) as usize;
        let filtered = m.filtered();
        let chip = format!(" {} \u{25b8} {}\u{258f} ", m.title, m.filter);
        let tip = format!(
            " \u{2191}\u{2193} \u{b7} \u{23ce} save \u{b7} esc \u{b7} {}/{} ",
            (m.cursor + 1).min(filtered.len()),
            filtered.len()
        );
        let mut rows = Vec::new();
        rows.push(status::rule_row(Some(&chip), true, Some(&tip), cols));
        // Cursor pinned near the bottom of the window; list slides.
        let top = (m.cursor + 1).saturating_sub(n_items);
        for row in 0..n_items {
            let idx = top + row;
            let line = match filtered.get(idx) {
                Some(name) => {
                    let star = if Some(*name) == engine_model.as_deref() {
                        "*"
                    } else {
                        ""
                    };
                    format!(" {name}{star}")
                }
                None if idx == 0 && !m.loaded => " probing \u{2026} (esc backs out)".to_string(),
                None if idx == 0 => " (no matches)".to_string(),
                None => String::new(),
            };
            let sgr = if idx == m.cursor && filtered.get(idx).is_some() {
                status::SUGGEST_SGR
            } else {
                status::TEXT_SGR
            };
            rows.push(status::pad_row(&line, cols, sgr));
        }
        rows.push(status::chrome_row(
            layout.real,
            inner_now,
            reserved_now,
            shell_name,
            sense::label(st, hook),
        ));
        return rows;
    }
    // While browsing the slot history, the browsed turn owns the area:
    // its command in the chip, its chat text in the band, position on
    // the rule's right end. Everything else is frozen underneath.
    let browsed = browse.and_then(|i| sug_hist.get(i).map(|t| (i, t)));
    let sug_chip = match browsed {
        Some((_, t)) => Some(format!(" \u{2193} suggestion: {} ", t.cmd)),
        None => suggestions
            .first()
            .map(|s| format!(" \u{2193} suggestion: {} ", s.1)),
    };
    let rule_text = sug_chip
        .clone()
        .or_else(|| notice.clone().map(|n| format!(" {n} ")));
    let reserved = cfg.reserved_rows();
    let inner_rows = layout.real.rows.saturating_sub(reserved).max(1);
    let label = sense::label(st, hook);

    if !cfg.status.band {
        // Minimal mode: a single chrome row.
        return vec![status::chrome_row(
            layout.real,
            inner_rows,
            reserved,
            shell_name,
            label,
        )];
    }

    let n_text = cfg.status.band_rows.clamp(1, 4);
    let mut rows = Vec::new();
    // Right end of the rule: scroll position while browsing the slot
    // history; otherwise the ingress tip — until a pullable suggestion
    // exists (the command is the more important thing).
    let tip = match browsed {
        Some((i, _)) => Some(format!(
            " \u{2191} {}/{}{} ",
            i + 1,
            sug_hist.len(),
            if i + 1 < sug_hist.len() {
                " \u{2193}"
            } else {
                ""
            }
        )),
        None if sug_chip.is_none() => {
            Some(" # message to chat \u{b7} #/help for help ".to_string())
        }
        None => None,
    };
    rows.push(status::rule_row(
        rule_text.as_deref(),
        sug_chip.is_some(),
        tip.as_deref(),
        cols,
    ));
    let q = match browsed {
        Some(_) => "suggestion history",
        None => band
            .as_ref()
            .and_then(|b| b.question.as_deref())
            .unwrap_or(""),
    };
    rows.push(status::pad_row(&format!(" {q}"), cols, status::QUERY_SGR));
    let mut lines = match browsed {
        Some((_, t)) => wrap_chars(&t.text, cols.saturating_sub(2), n_text as usize),
        None => band
            .as_ref()
            .map(|b| wrap_chars(&b.text, cols.saturating_sub(2), n_text as usize))
            .unwrap_or_default(),
    };
    while (lines.len() as u16) < n_text {
        lines.push(String::new());
    }
    for line in lines {
        rows.push(status::pad_row(&format!(" {line}"), cols, status::TEXT_SGR));
    }
    rows.push(status::chrome_row(
        layout.real,
        inner_rows,
        reserved,
        shell_name,
        label,
    ));
    rows
}

/// Apply a reserved-row-count change: shrink/grow the inner PTY (the
/// band opening and closing is just more winsize arithmetic) and return
/// clear-bytes for any rows handed back to the inner world.
fn sync_reserved(
    layout: &mut Layout,
    parser: &mut vt100::Parser,
    master: RawFd,
    new_reserved: u16,
) -> Vec<u8> {
    let mut pre = Vec::new();
    if new_reserved == layout.reserved {
        return pre;
    }
    let old_inner = layout.inner().rows;
    layout.reserved = new_reserved;
    let inner = layout.inner();
    parser.screen_mut().set_size(inner.rows, inner.cols);
    let _ = term::set_size(master, inner);
    for r in (old_inner + 1)..=inner.rows {
        // Rows reclaimed from the band back to the shell: clear leftovers.
        pre.extend_from_slice(format!("\x1b[{r};1H\x1b[K").as_bytes());
    }
    pre
}

/// Scroll-region assertion + reserved-row redraw + cursor/attribute
/// restore, derived from the tracked inner-screen state.
fn fixup_bytes(layout: &Layout, screen: &vt100::Screen, rows: &[String]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::with_capacity(512);
    let inner = layout.inner();
    out.extend_from_slice(b"\x1b[?25l"); // hide cursor while we work
    out.extend_from_slice(format!("\x1b[1;{}r", inner.rows).as_bytes());
    for (i, row) in rows.iter().enumerate() {
        out.extend_from_slice(format!("\x1b[{};1H", inner.rows + 1 + i as u16).as_bytes());
        out.extend_from_slice(row.as_bytes());
    }
    out.extend_from_slice(&screen.attributes_formatted());
    out.extend_from_slice(&screen.cursor_state_formatted());
    out
}

/// `#/` command dispatch over the full command line (memory verbs take
/// free text). Returns the bar notice to show.
#[allow(clippy::too_many_arguments)]
fn slash_command(
    cmdline: &str,
    engine: Option<&Engine>,
    engine_model: &Option<String>,
    blocks: u64,
    commentary: &mut bool,
    memory: &mut MemoryStore,
    fuse: &mut StateFile,
    menu: &mut Option<Menu>,
) -> Option<String> {
    let mut it = cmdline.splitn(2, char::is_whitespace);
    let cmd = it.next().unwrap_or("");
    let rest = it.next().unwrap_or("").trim();
    let arg = if rest.is_empty() { None } else { Some(rest) };
    match (cmd, arg) {
        ("memory", sub) => Some(memory_command(sub, memory)),
        ("commentary", arg) => {
            let arg = arg.map(|a| a.split_whitespace().next().unwrap_or(a));
            *commentary = match arg {
                Some("on") | Some("true") => true,
                Some("off") | Some("false") => false,
                _ => !*commentary,
            };
            Some(format!(
                "commentary {}",
                if *commentary { "on" } else { "off" }
            ))
        }
        ("model", Some(rest)) => {
            let mut t = rest.split_whitespace();
            let name = t.next().unwrap_or(rest);
            let save = t.next() == Some("save");
            match engine {
                Some(eng) if name == "auto" => {
                    eng.rebind();
                    Some(if save {
                        match Config::persist_model(None) {
                            Ok(()) => "auto \u{2192} default (probe order restored)".to_string(),
                            Err(e) => format!("config write failed: {e}"),
                        }
                    } else {
                        "re-probing (auto) \u{2026}".to_string()
                    })
                }
                Some(eng) => {
                    eng.set_model(name.to_string());
                    Some(if save {
                        match Config::persist_model(Some(name)) {
                            Ok(()) => {
                                fuse.set_probation(name);
                                format!(
                                    "model {name} \u{2192} default \
                                     (probation until first answer)"
                                )
                            }
                            Err(e) => format!("config write failed: {e}"),
                        }
                    } else {
                        format!("switching model to {name} \u{2026}")
                    })
                }
                None => Some("no engine running".to_string()),
            }
        }
        ("model", None) => match engine {
            Some(eng) => {
                // Bare form opens the modal selector; the list fills in
                // async when the probe answers (never block the shell).
                eng.list_models();
                *menu = Some(Menu::open("model"));
                None
            }
            None => Some("no engine running".to_string()),
        },
        ("status", _) => Some(format!(
            "goulash {} \u{b7} engine: {} \u{b7} {} blocks this session",
            env!("CARGO_PKG_VERSION"),
            engine_model.as_deref().unwrap_or("none"),
            blocks,
        )),
        ("help", _) => Some(
            "#/model [name [save]] \u{b7} #/commentary [on|off] \u{b7} #/memory \u{2026} \u{b7} \
             #/status"
                .to_string(),
        ),
        _ => Some(format!("unknown command /{cmd} \u{2014} try #/help")),
    }
}

/// `#/memory on|off|limit N|add TEXT|delete ID|modify ID TEXT|find Q`
fn memory_command(sub: Option<&str>, memory: &mut MemoryStore) -> String {
    let sub = sub.unwrap_or("");
    let mut it = sub.splitn(2, char::is_whitespace);
    let verb = it.next().unwrap_or("");
    let rest = it.next().unwrap_or("").trim();
    match verb {
        "" => memory.status_line(),
        "on" | "true" => {
            memory.set_enabled(true);
            memory.status_line()
        }
        "off" | "false" => {
            memory.set_enabled(false);
            memory.status_line()
        }
        "limit" => match rest.parse::<usize>() {
            Ok(n) => {
                memory.set_limit(n);
                memory.status_line()
            }
            Err(_) => "usage: #/memory limit <n>".to_string(),
        },
        "add" => match memory.add(rest, "user") {
            Ok(id) => format!("remembered [{id}] \u{b7} {}", memory.status_line()),
            Err(e) => e,
        },
        "delete" | "forget" => match rest.parse::<u64>() {
            Ok(id) if memory.delete(id) => format!("forgot [{id}]"),
            Ok(id) => format!("no memory [{id}]"),
            Err(_) => "usage: #/memory delete <id>".to_string(),
        },
        "modify" => {
            let mut p = rest.splitn(2, char::is_whitespace);
            match (p.next().and_then(|s| s.parse::<u64>().ok()), p.next()) {
                (Some(id), Some(text)) if memory.modify(id, text) => {
                    format!("updated [{id}]")
                }
                (Some(id), Some(_)) => format!("no memory [{id}]"),
                _ => "usage: #/memory modify <id> <text>".to_string(),
            }
        }
        "find" | "list" => {
            let hits = memory.find(rest);
            if hits.is_empty() {
                "no matching memories".to_string()
            } else {
                hits.iter()
                    .map(|s| format!("[{}] {}", s.id, s.text))
                    .collect::<Vec<_>>()
                    .join(" \u{b7} ")
            }
        }
        _ => "usage: #/memory on|off|limit|add|delete|modify|find".to_string(),
    }
}

pub fn run(cfg: &Config, argv: Vec<String>) -> io::Result<i32> {
    let real = term::get_size(STDOUT)?;
    if real.rows < 4 || real.cols < 10 {
        return Err(io::Error::other("terminal too small"));
    }
    let mut layout = Layout {
        real,
        reserved: cfg.reserved_rows(),
    };
    let shell_name = std::path::Path::new(&argv[0])
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| argv[0].clone());

    let integ = crate::integrate::prepare(argv.clone(), cfg.shell.auto_integrate);
    let mut p = pty::spawn(&integ.argv, &integ.envs, layout.inner())?;
    let master = p.master.as_raw_fd();

    let raw = RawGuard::new(unsafe { BorrowedFd::borrow_raw(STDIN) })
        .map_err(|e| io::Error::other(format!("raw mode: {e}")))?;

    // SIGWINCH self-pipe.
    let (winch_rd, winch_wr) = nix::unistd::pipe()?;
    WINCH_PIPE_WR.store(winch_wr.as_raw_fd(), Ordering::Relaxed);
    let action = SigAction::new(
        SigHandler::Handler(on_sigwinch),
        SaFlags::SA_RESTART,
        SigSet::empty(),
    );
    // SAFETY: handler only does an async-signal-safe write to the pipe.
    unsafe { signal::sigaction(Signal::SIGWINCH, &action) }
        .map_err(|e| io::Error::other(format!("sigaction: {e}")))?;

    // Place existing screen content clear of the reserved rows, without
    // clearing the screen or entering the alternate screen: scroll just
    // enough that the cursor lands inside the inner region.
    let (row, col, typed_ahead) = query_cursor_row(layout.real);
    let inner_rows = layout.inner().rows;
    let mut init: Vec<u8> = Vec::new();
    let final_row = if row > inner_rows {
        init.extend(std::iter::repeat_n(b'\n', (row - inner_rows) as usize));
        inner_rows
    } else {
        row
    };
    init.extend_from_slice(format!("\x1b[1;{inner_rows}r").as_bytes());
    init.extend_from_slice(format!("\x1b[{final_row};{col}H").as_bytes());
    write_all(STDOUT, &init)?;

    let mut parser = vt100::Parser::new(inner_rows, layout.real.cols, 0);
    let sensor = Sensor::new(master, p.child.id());
    let mut rec = Recorder::new(cfg);
    rec.start(&argv, layout.real.rows, layout.real.cols);
    let mut cur_state: State = sensor.read(false);
    rec.state(cur_state);

    let mut oscf = OscFilter::new();
    let mut hook: Option<HookPhase> = None;
    let mut rules = RulesVendor::new();
    let mut suggestions: Vec<(u64, String, String)> = Vec::new();
    let mut sug_hist: Vec<SugTurn> = Vec::new();
    let mut browse: Option<usize> = None;
    let mut next_sid: u64 = 1;
    let mut cur_cmd: Option<String> = None;
    let mut block_tail: Vec<u8> = Vec::new();
    let mut last_cwd = String::new();
    // Crash fuse: refuse to auto-bind a model that took the last run
    // down mid-load/mid-generation; land on last_good (or auto) instead.
    let mut fuse = StateFile::load(Config::dir());
    let mut eng_cfg = cfg.engine.clone();
    let mut notice: Option<String> = None;
    if let Some(fallback) = fuse.veto(eng_cfg.model.as_deref()) {
        let bad = eng_cfg.model.take().unwrap_or_default();
        notice = Some(format!(
            "{bad} didn't survive its last run \u{2014} on {}; \
             '#/model {bad} save' to insist",
            fallback.as_deref().unwrap_or("auto")
        ));
        eng_cfg.model = fallback;
    }
    // Append-only session log for engine context: byte-stable prefix so
    // the provider's KV cache re-uses everything but the appended tail.
    // Epoch-trims at a block boundary when over budget (one cache miss).
    let mut ctx_log = String::new();
    let mut blocks_seen: u64 = 0;
    let mut engine: Option<Engine> = if cfg.engine.provider != "none" {
        Engine::start(eng_cfg).ok()
    } else {
        None
    };
    let mut engine_model: Option<String> = None;
    let mut commentary = cfg.engine.commentary;
    let mut memory = MemoryStore::load(Config::dir());
    let mut band: Option<Band> = None;
    let mut menu: Option<Menu> = None;
    let mut chat: Option<Chat> = None;
    let mut warming: Option<String> = None;
    #[allow(unused_assignments)] // initialized by the first redraw! below
    let mut last_rows: Vec<String> = Vec::new();
    macro_rules! redraw {
        () => {{
            let rows = compose_rows(
                cfg,
                &layout,
                &shell_name,
                &cur_state,
                hook,
                &suggestions,
                &notice,
                &band,
                browse,
                &sug_hist,
                &menu,
                &engine_model,
                &chat,
            );
            let pre = sync_reserved(&mut layout, &mut parser, master, rows.len() as u16);
            if !pre.is_empty() {
                write_all(STDOUT, &pre)?;
            }
            write_all(STDOUT, &fixup_bytes(&layout, parser.screen(), &rows))?;
            last_rows = rows;
        }};
    }
    redraw!();
    if !typed_ahead.is_empty() {
        let _ = write_all(master, &typed_ahead);
    }

    let mut buf = [0u8; 65536];
    let mut stdin_open = true;
    let mut dirty = false;
    let mut idle_ticks: u8 = 0;

    'session: loop {
        let stdin_fd = unsafe { BorrowedFd::borrow_raw(STDIN) };
        let master_fd = unsafe { BorrowedFd::borrow_raw(master) };
        let mut fds: Vec<PollFd> = Vec::with_capacity(4);
        fds.push(PollFd::new(master_fd, PollFlags::POLLIN));
        fds.push(PollFd::new(winch_rd.as_fd(), PollFlags::POLLIN));
        let engine_idx = if let Some(eng) = engine.as_ref() {
            fds.push(PollFd::new(eng.wake.as_fd(), PollFlags::POLLIN));
            Some(fds.len() - 1)
        } else {
            None
        };
        let stdin_idx = if stdin_open {
            fds.push(PollFd::new(stdin_fd, PollFlags::POLLIN));
            Some(fds.len() - 1)
        } else {
            None
        };
        // Tick at 250ms even when idle so job-control transitions with no
        // accompanying I/O (e.g. `sleep 5` starting) are still sensed;
        // 30ms while dirty for quiescence-debounced redraws.
        let timeout = PollTimeout::try_from(if dirty { 30 } else { 250 }).unwrap();
        let n = match nix::poll::poll(&mut fds, timeout) {
            Ok(n) => n,
            Err(nix::errno::Errno::EINTR) => continue,
            Err(e) => return Err(io::Error::other(format!("poll: {e}"))),
        };

        let master_ready = n > 0 && fds[0].revents().is_some_and(|r| !r.is_empty());
        let winch_ready = n > 0
            && fds[1]
                .revents()
                .is_some_and(|r| r.contains(PollFlags::POLLIN));
        let engine_revents = if n > 0 {
            engine_idx
                .and_then(|i| fds.get(i))
                .and_then(|f| f.revents())
        } else {
            None
        };
        let engine_ready_fd = engine_revents.is_some_and(|r| r.contains(PollFlags::POLLIN));
        let engine_hup =
            engine_revents.is_some_and(|r| r.intersects(PollFlags::POLLHUP | PollFlags::POLLERR));
        let stdin_ready = n > 0
            && stdin_idx
                .and_then(|i| fds.get(i))
                .and_then(|f| f.revents())
                .is_some_and(|r| !r.is_empty());

        if master_ready {
            match read_some(master, &mut buf) {
                Ok(0) => break 'session,
                Ok(len) => {
                    // Cleaned stream and marks arrive as ordered segments,
                    // so output is attributed to the correct command block
                    // even when B/output/D land in a single read.
                    let mut trigger_seen = false;
                    for seg in oscf.feed(&buf[..len]) {
                        match seg {
                            Seg::Bytes(bytes) => {
                                if cur_cmd.is_some() {
                                    block_tail.extend_from_slice(&bytes);
                                    let excess = block_tail.len().saturating_sub(8192);
                                    if excess > 0 {
                                        block_tail.drain(..excess);
                                    }
                                }
                                rec.output(&bytes);
                                // Re-pin the scroll region at trigger
                                // boundaries (see find_trigger_end).
                                let mut rest: &[u8] = &bytes;
                                while let Some(end) = find_trigger_end(rest) {
                                    let (pre, post) = rest.split_at(end);
                                    parser.process(pre);
                                    write_all(STDOUT, pre)?;
                                    let mut fix =
                                        format!("\x1b[1;{}r", layout.inner().rows).into_bytes();
                                    fix.extend_from_slice(
                                        &parser.screen().cursor_state_formatted(),
                                    );
                                    write_all(STDOUT, &fix)?;
                                    trigger_seen = true;
                                    rest = post;
                                }
                                parser.process(rest);
                                write_all(STDOUT, rest)?;
                            }
                            Seg::Mark(m) => match m {
                                Mark::Prompt => {
                                    hook = Some(HookPhase::Prompt);
                                    rec.prompt();
                                }
                                Mark::CmdStart(cmd) => {
                                    hook = Some(HookPhase::Command);
                                    rec.cmd_start(&cmd);
                                    cur_cmd = Some(cmd);
                                    block_tail.clear();
                                    notice = None;
                                    band = None;
                                    browse = None;
                                }
                                Mark::CmdEnd(code) => {
                                    rec.cmd_end(code);
                                    if code == 0 {
                                        // Whatever was broken got fixed (or
                                        // moved past): drop stale fixes.
                                        suggestions.clear();
                                    }
                                    if let Some(cmd) = cur_cmd.take() {
                                        let block = vendor::CmdBlock {
                                            cmd,
                                            exit_code: code,
                                            cwd: last_cwd.clone(),
                                            output_tail: String::from_utf8_lossy(&block_tail)
                                                .into_owned(),
                                        };
                                        blocks_seen += 1;
                                        ctx_log.push_str(&format!(
                                            "$ {} [exit {}, {}]\n",
                                            block.cmd,
                                            block.exit_code,
                                            engine::hms()
                                        ));
                                        let tail: String = block
                                            .output_tail
                                            .chars()
                                            .take(cfg.engine.tail_chars)
                                            .collect();
                                        if !tail.trim().is_empty() {
                                            ctx_log.push_str(tail.trim());
                                            ctx_log.push('\n');
                                        }
                                        if ctx_log.len() > cfg.engine.context_max_chars {
                                            let keep = cfg.engine.context_max_chars / 2;
                                            let mut start = ctx_log.len().saturating_sub(keep);
                                            while !ctx_log.is_char_boundary(start) {
                                                start += 1;
                                            }
                                            let cut = ctx_log[start..]
                                                .find("\n$ ")
                                                .map(|p| start + p + 1)
                                                .unwrap_or(start);
                                            ctx_log.drain(..cut);
                                        }
                                        for v in rules.suggest(&block) {
                                            let id = next_sid;
                                            next_sid += 1;
                                            rec.suggest(id, &v.command, &v.why, v.vendor);
                                            hist_push(
                                                &mut sug_hist,
                                                SugTurn {
                                                    id,
                                                    cmd: v.command.clone(),
                                                    text: v.why.clone(),
                                                },
                                            );
                                            suggestions.insert(0, (id, v.command, v.why));
                                        }
                                        suggestions.truncate(8);
                                        if commentary
                                            && engine_model.is_some()
                                            && let Some(eng) = engine.as_ref()
                                        {
                                            eng.ask_proactive(
                                                ctx_log.clone(),
                                                memory.context_block(),
                                            );
                                        }
                                    }
                                }
                                Mark::Cwd(p) => {
                                    if !last_cwd.is_empty() && p != last_cwd {
                                        // cwd changed: context moved, old
                                        // suggestions are stale.
                                        suggestions.clear();
                                        ctx_log.push_str(&format!("[cwd: {p}]\n"));
                                    }
                                    rec.cwd(&p);
                                    last_cwd = p;
                                }
                                Mark::Ask(q) => {
                                    rec.aside(&q);
                                    browse = None;
                                    let body = q.trim_start_matches('#').trim();
                                    if q.starts_with("##") {
                                        // `##` flips the script: chat has
                                        // focus. A body rides along as the
                                        // first message.
                                        let mut c = Chat {
                                            lines: Vec::new(),
                                            input: String::new(),
                                            stream: None,
                                            sel: None,
                                        };
                                        if !body.is_empty()
                                            && let Some(eng) = engine.as_ref()
                                        {
                                            c.lines.push(format!("# {body}"));
                                            eng.ask(
                                                body.to_string(),
                                                ctx_log.clone(),
                                                memory.context_block(),
                                            );
                                            ctx_log.push_str(&format!(
                                                "# {} [asked {}]\n",
                                                body,
                                                engine::hms()
                                            ));
                                        }
                                        band = None;
                                        notice = None;
                                        chat = Some(c);
                                    } else if let Some(cmdline) = body.strip_prefix('/') {
                                        // #/ commands: goulash controls, not
                                        // LLM asides. One arg max — the
                                        // single most obvious swivel.
                                        notice = slash_command(
                                            cmdline,
                                            engine.as_ref(),
                                            &engine_model,
                                            blocks_seen,
                                            &mut commentary,
                                            &mut memory,
                                            &mut fuse,
                                            &mut menu,
                                        );
                                    } else if let Some(eng) = engine.as_ref() {
                                        eng.ask(
                                            body.to_string(),
                                            ctx_log.clone(),
                                            memory.context_block(),
                                        );
                                        ctx_log.push_str(&format!(
                                            "# {} [asked {}]\n",
                                            body,
                                            engine::hms()
                                        ));
                                        notice = None;
                                        band = Some(Band {
                                            question: Some(q.clone()),
                                            text: "\u{2026}".to_string(),
                                        });
                                    } else {
                                        notice =
                                            Some(format!("{q} \u{2014} no engine configured yet"));
                                    }
                                }
                                Mark::Pull(buffer) => {
                                    // Slot history: a single-slot scrollable
                                    // view over past (suggestion, chat)
                                    // turns. Empty buffer enters at the
                                    // newest; a buffer that IS the current
                                    // slot steps one older (kill line,
                                    // repaste) and STOPS at the oldest.
                                    // The user's own text is never
                                    // clobbered — a mismatch ends browsing.
                                    let target = if sug_hist.is_empty() {
                                        browse = None;
                                        None
                                    } else if buffer.is_empty() {
                                        Some(0)
                                    } else {
                                        let pos = browse
                                            .filter(|&p| {
                                                sug_hist.get(p).map(|t| t.cmd == buffer)
                                                    == Some(true)
                                            })
                                            .or_else(|| {
                                                sug_hist.iter().position(|t| t.cmd == buffer)
                                            });
                                        match pos {
                                            Some(p) => Some((p + 1).min(sug_hist.len() - 1)),
                                            None => {
                                                browse = None;
                                                None
                                            }
                                        }
                                    };
                                    if let Some(i) = target {
                                        let turn = sug_hist[i].clone();
                                        if turn.cmd != buffer {
                                            rec.accept(turn.id);
                                            let mut bytes = Vec::new();
                                            if !buffer.is_empty() {
                                                bytes.push(0x15); // ^U: kill line
                                            }
                                            bytes.extend_from_slice(b"\x1b[200~");
                                            bytes.extend_from_slice(turn.cmd.as_bytes());
                                            bytes.extend_from_slice(b"\x1b[201~");
                                            write_all(master, &bytes)?;
                                        } else {
                                            // At the oldest: resolve the
                                            // shell's paste-expect anyway.
                                            write_all(master, b"\x1b[200~\x1b[201~")?;
                                        }
                                        browse = Some(i);
                                    } else {
                                        write_all(master, b"\x1b[200~\x1b[201~")?;
                                    }
                                }
                                Mark::PullUp(buffer) => {
                                    // The same axis, other direction: Up
                                    // slides toward the neutral empty line
                                    // (zsh history resumes above it). Empty
                                    // pastes resolve the shell's tracking
                                    // on every path.
                                    let pos = browse
                                        .filter(|&p| {
                                            sug_hist.get(p).map(|t| t.cmd == buffer) == Some(true)
                                        })
                                        .or_else(|| sug_hist.iter().position(|t| t.cmd == buffer));
                                    match pos {
                                        Some(0) => {
                                            write_all(master, b"\x15\x1b[200~\x1b[201~")?;
                                            browse = None;
                                        }
                                        Some(i) => {
                                            let turn = sug_hist[i - 1].clone();
                                            rec.accept(turn.id);
                                            let mut bytes = vec![0x15];
                                            bytes.extend_from_slice(b"\x1b[200~");
                                            bytes.extend_from_slice(turn.cmd.as_bytes());
                                            bytes.extend_from_slice(b"\x1b[201~");
                                            write_all(master, &bytes)?;
                                            browse = Some(i - 1);
                                        }
                                        None => {
                                            write_all(master, b"\x1b[200~\x1b[201~")?;
                                            browse = None;
                                        }
                                    }
                                }
                            },
                        }
                    }
                    if trigger_seen {
                        redraw!();
                        dirty = false;
                    } else {
                        dirty = true;
                    }
                }
                Err(e) if e.raw_os_error() == Some(libc::EIO) => break 'session,
                Err(e) => return Err(e),
            }
        }

        if winch_ready {
            let mut drain = [0u8; 64];
            let _ = read_some(winch_rd.as_raw_fd(), &mut drain);
            if let Ok(real) = term::get_size(STDOUT) {
                layout.real = real;
                let inner = layout.inner();
                parser.screen_mut().set_size(inner.rows, inner.cols);
                let _ = term::set_size(master, inner);
                rec.resize(real.rows, real.cols);
                redraw!();
                dirty = false;
            }
        }

        if stdin_ready {
            match read_some(STDIN, &mut buf) {
                Ok(0) => stdin_open = false,
                Ok(len) if menu.is_some() => {
                    // Modal menu: goulash owns the keyboard — the one
                    // exception to shell-owns-input, and only because the
                    // user opened it by name. Typing filters (no hotkeys);
                    // Enter commits AND persists; Esc/Ctrl-C always out.
                    let mut committed: Option<String> = None;
                    let mut close = false;
                    if let Some(m) = menu.as_mut() {
                        for key in parse_keys(&buf[..len]) {
                            match key {
                                Key::Esc | Key::CtrlC => close = true,
                                Key::Enter => {
                                    committed = m.filtered().get(m.cursor).map(|s| s.to_string());
                                    close = true;
                                }
                                Key::Up => m.cursor = m.cursor.saturating_sub(1),
                                Key::Down => {
                                    m.cursor += 1;
                                    m.clamp();
                                }
                                Key::Backspace => {
                                    m.filter.pop();
                                    m.clamp();
                                }
                                Key::KillLine => {
                                    m.filter.clear();
                                    m.cursor = 0;
                                }
                                Key::Char(c) => {
                                    m.filter.push(c);
                                    m.cursor = 0;
                                }
                            }
                        }
                    }
                    if let Some(name) = committed
                        && let Some(eng) = engine.as_ref()
                    {
                        notice = Some(if name == "auto" {
                            eng.rebind();
                            match Config::persist_model(None) {
                                Ok(()) => {
                                    "auto \u{2192} default (probe order restored)".to_string()
                                }
                                Err(e) => format!("config write failed: {e}"),
                            }
                        } else {
                            eng.set_model(name.clone());
                            match Config::persist_model(Some(&name)) {
                                Ok(()) => {
                                    fuse.set_probation(&name);
                                    format!(
                                        "model {name} \u{2192} default \
                                         (probation until first answer)"
                                    )
                                }
                                Err(e) => format!("config write failed: {e}"),
                            }
                        });
                    }
                    if close {
                        menu = None;
                    }
                    dirty = true;
                }
                Ok(len) if chat.is_some() => {
                    // Chat has focus: a real (if minimal) input line.
                    // Commands still exit through the shell — Up hands
                    // the newest suggestion over and focus flips back.
                    let mut exit_chat = false;
                    let mut submit: Option<String> = None;
                    let mut handoff: Option<usize> = None;
                    if let Some(c) = chat.as_mut() {
                        for key in parse_keys(&buf[..len]) {
                            match key {
                                // Esc backs out one layer: selection →
                                // input → shell.
                                Key::Esc | Key::CtrlC => {
                                    if c.sel.is_some() {
                                        c.sel = None;
                                    } else {
                                        exit_chat = true;
                                    }
                                }
                                Key::Enter => {
                                    if let Some(i) = c.sel {
                                        handoff = Some(i);
                                        c.sel = None;
                                    } else {
                                        let text = c.input.trim().to_string();
                                        c.input.clear();
                                        if text == "##" {
                                            exit_chat = true;
                                        } else if !text.is_empty() {
                                            submit = Some(text);
                                        }
                                    }
                                }
                                // Same axis as the prompt: Down dives
                                // older through the slot stack, Up walks
                                // back newer to neutral — and Up at
                                // neutral grabs the newest directly.
                                Key::Down if c.input.is_empty() && !sug_hist.is_empty() => {
                                    c.sel = Some(match c.sel {
                                        None => 0,
                                        Some(i) => (i + 1).min(sug_hist.len() - 1),
                                    });
                                }
                                Key::Up if c.input.is_empty() => match c.sel {
                                    Some(0) => c.sel = None,
                                    Some(i) => c.sel = Some(i - 1),
                                    None => {}
                                },
                                Key::Backspace => {
                                    c.input.pop();
                                }
                                Key::KillLine => c.input.clear(),
                                Key::Char(ch) => {
                                    c.sel = None; // typing returns to input
                                    c.input.push(ch);
                                }
                                _ => {}
                            }
                        }
                    }
                    if let Some(text) = submit {
                        rec.aside(&format!("## {text}"));
                        if let Some(cmdline) = text.strip_prefix('/') {
                            let out = slash_command(
                                cmdline,
                                engine.as_ref(),
                                &engine_model,
                                blocks_seen,
                                &mut commentary,
                                &mut memory,
                                &mut fuse,
                                &mut menu,
                            );
                            if let (Some(c), Some(msg)) = (chat.as_mut(), out) {
                                c.lines.push(format!("goulash: {msg}"));
                            }
                        } else if let Some(eng) = engine.as_ref() {
                            if let Some(c) = chat.as_mut() {
                                c.lines.push(format!("# {text}"));
                            }
                            eng.ask(text.clone(), ctx_log.clone(), memory.context_block());
                            ctx_log.push_str(&format!("# {} [asked {}]\n", text, engine::hms()));
                        } else if let Some(c) = chat.as_mut() {
                            c.lines
                                .push("goulash: no engine configured yet".to_string());
                        }
                    }
                    if let Some(i) = handoff
                        && let Some(turn) = sug_hist.get(i).cloned()
                    {
                        // Keep it pure: the command lands on the real
                        // shell line for the user's own editor + Enter.
                        rec.accept(turn.id);
                        let mut bytes = Vec::new();
                        bytes.extend_from_slice(b"\x1b[200~");
                        bytes.extend_from_slice(turn.cmd.as_bytes());
                        bytes.extend_from_slice(b"\x1b[201~");
                        write_all(master, &bytes)?;
                        browse = Some(i);
                        exit_chat = true;
                    }
                    if exit_chat {
                        chat = None;
                    }
                    dirty = true;
                }
                Ok(len) => {
                    // Alt-Down: generic-shell suggestion pull. Only ever
                    // intercepted at a hook-confirmed prompt with a live
                    // suggestion; everything else passes through verbatim.
                    const ALT_DOWN: &[u8] = b"\x1b[1;3B";
                    let chunk = &buf[..len];
                    let pos = if hook == Some(HookPhase::Prompt) && !suggestions.is_empty() {
                        chunk.windows(ALT_DOWN.len()).position(|w| w == ALT_DOWN)
                    } else {
                        None
                    };
                    if let Some(p) = pos {
                        write_all(master, &chunk[..p])?;
                        let (id, cmdtext, _why) = suggestions[0].clone();
                        rec.accept(id);
                        let mut paste = Vec::new();
                        paste.extend_from_slice(b"\x1b[200~");
                        paste.extend_from_slice(cmdtext.as_bytes());
                        paste.extend_from_slice(b"\x1b[201~");
                        write_all(master, &paste)?;
                        write_all(master, &chunk[p + ALT_DOWN.len()..])?;
                        dirty = true;
                    } else {
                        write_all(master, chunk)?;
                    }
                }
                Err(_) => stdin_open = false,
            }
        }

        if engine_hup && !engine_ready_fd {
            // Worker died (panic or clean end): stop polling its pipe.
            engine = None;
            engine_model = None;
        }

        if engine_ready_fd && let Some(eng) = engine.as_ref() {
            let mut drain = [0u8; 64];
            let _ = read_some(eng.wake.as_raw_fd(), &mut drain);
            while let Ok(ev) = eng.events.try_recv() {
                match ev {
                    engine::Event::Ready { provider, model } => {
                        rec.engine_ready(&provider, &model);
                        notice = Some(format!("engine: {provider} \u{b7} {model}"));
                        engine_model = Some(model);
                    }
                    engine::Event::Partial(text) => {
                        if let Some(c) = chat.as_mut() {
                            c.stream = Some(text.split_whitespace().collect::<Vec<_>>().join(" "));
                        } else {
                            match band.as_mut() {
                                Some(b) => b.text = format!("{text} \u{2026}"),
                                None => {
                                    let one_line =
                                        text.split_whitespace().collect::<Vec<_>>().join(" ");
                                    notice = Some(format!("{one_line} \u{2026}"));
                                }
                            }
                        }
                    }
                    engine::Event::Answer {
                        text,
                        command,
                        proactive,
                        remembers,
                        forgets,
                    } => {
                        // The generation completed: the bound model earned
                        // its trust (ends probation, clears any distrust).
                        if let Some(m) = engine_model.as_ref() {
                            fuse.promote(m);
                        }
                        if memory.enabled {
                            // Forgets first: a modify is FORGET + REMEMBER in
                            // one reply, and the delete must free the slot
                            // before the add when the store is full.
                            for id in &forgets {
                                if memory.delete(*id) {
                                    rec.memory("forget", *id, "");
                                }
                            }
                            for note in &remembers {
                                if let Ok(id) = memory.add(note, "llm") {
                                    rec.memory("add", id, note);
                                }
                            }
                        }
                        let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
                        let passed = proactive
                            && one_line
                                .trim_matches(['.', '!'])
                                .eq_ignore_ascii_case("PASS");
                        // A proactive tip never overwrites a user ask's band.
                        let user_band_active =
                            band.as_ref().map(|b| b.question.is_some()).unwrap_or(false);
                        if passed || (proactive && user_band_active) {
                            rec.aside_answer(&one_line, true);
                        } else {
                            rec.aside_answer(&one_line, true);
                            ctx_log.push_str(&format!("goulash: {one_line}\n"));
                            if let Some(cmd) = command {
                                let id = next_sid;
                                next_sid += 1;
                                let why = if proactive {
                                    "commentary"
                                } else {
                                    "from # ask"
                                };
                                rec.suggest(id, &cmd, why, "engine");
                                hist_push(
                                    &mut sug_hist,
                                    SugTurn {
                                        id,
                                        cmd: cmd.clone(),
                                        text: one_line.clone(),
                                    },
                                );
                                suggestions.insert(0, (id, cmd.clone(), why.to_string()));
                                suggestions.truncate(8);
                                ctx_log.push_str(&format!("CMD: {cmd}\n"));
                            }
                            if let Some(c) = chat.as_mut() {
                                c.stream = None;
                                c.lines.push(format!("goulash: {one_line}"));
                            } else if proactive {
                                band = Some(Band {
                                    question: None,
                                    text,
                                });
                            } else {
                                match band.as_mut() {
                                    Some(b) => b.text = text,
                                    None => notice = Some(one_line),
                                }
                            }
                        }
                    }
                    engine::Event::Error(msg) => {
                        rec.aside_answer(&msg, false);
                        let m = format!("engine error: {msg}");
                        if let Some(c) = chat.as_mut() {
                            c.stream = None;
                            c.lines.push(format!("goulash: {m}"));
                        } else {
                            match band.as_mut() {
                                Some(b) => b.text = m,
                                None => notice = Some(m),
                            }
                        }
                    }
                    engine::Event::Debug(raw) => rec.engine_debug(&raw),
                    engine::Event::Busy { model, warm } => {
                        fuse.busy(&model);
                        if warm {
                            // A model load can take a long minute on a
                            // big model — never leave the user pinned
                            // and guessing.
                            notice = Some(format!("loading {model} \u{2026}"));
                            warming = Some(model);
                        }
                    }
                    engine::Event::Idle => {
                        fuse.idle();
                        if let Some(m) = warming.take() {
                            notice = Some(format!("{m} ready"));
                        }
                    }
                    engine::Event::Models(names) => match menu.as_mut() {
                        Some(m) if !m.loaded => {
                            // "auto" is a first-class entry: it restores
                            // the probe chain and clears the pin.
                            m.items = std::iter::once("auto".to_string())
                                .chain(names.iter().cloned())
                                .collect();
                            m.loaded = true;
                        }
                        _ => {
                            let list = names
                                .iter()
                                .map(|n| {
                                    if Some(n) == engine_model.as_ref() {
                                        format!("{n}*")
                                    } else {
                                        n.clone()
                                    }
                                })
                                .collect::<Vec<_>>()
                                .join(" \u{b7} ");
                            notice = Some(format!("models: {list}"));
                        }
                    },
                }
                dirty = true;
            }
        }

        // Sense job-control / termios / alt-screen transitions.
        let st = sensor.read(parser.screen().alternate_screen());
        if st != cur_state {
            cur_state = st;
            rec.state(st);
            // A fullscreen app took the inner world: chat focus yields
            // (vim needs the keyboard more than we do).
            if cur_state.alt_screen && chat.is_some() {
                chat = None;
            }
            dirty = true;
        }

        // Quiescent and dirty: redraw the status row — but only if its
        // content actually changed. Ordinary output can't touch the bar
        // (the inner world is winsize-fenced; bar-threatening sequences
        // are handled as same-batch triggers above), so skipping no-op
        // redraws avoids needless writes. As insurance against trigger
        // sequences we don't know about, an idle repaint runs about once
        // a second — overwriting an intact bar with identical content is
        // invisible, so this costs nothing visually.
        if n == 0 {
            if dirty {
                let rows = compose_rows(
                    cfg,
                    &layout,
                    &shell_name,
                    &cur_state,
                    hook,
                    &suggestions,
                    &notice,
                    &band,
                    browse,
                    &sug_hist,
                    &menu,
                    &engine_model,
                    &chat,
                );
                if rows != last_rows {
                    let pre = sync_reserved(&mut layout, &mut parser, master, rows.len() as u16);
                    if !pre.is_empty() {
                        write_all(STDOUT, &pre)?;
                    }
                    write_all(STDOUT, &fixup_bytes(&layout, parser.screen(), &rows))?;
                    last_rows = rows;
                }
                dirty = false;
                idle_ticks = 0;
            } else {
                idle_ticks += 1;
                if idle_ticks >= 4 {
                    idle_ticks = 0;
                    write_all(STDOUT, &fixup_bytes(&layout, parser.screen(), &last_rows))?;
                }
            }
        }
    }

    // Restore the terminal: full scroll region, cursor to the old status
    // row start, default attributes, visible cursor.
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(b"\x1b[r");
    out.extend_from_slice(format!("\x1b[{};1H", layout.status_row()).as_bytes());
    out.extend_from_slice(b"\x1b[0m\x1b[2K\x1b[?25h");
    let _ = write_all(STDOUT, &out);
    drop(raw);
    WINCH_PIPE_WR.store(-1, Ordering::Relaxed);

    let st = p.child.wait()?;
    let code = st.code().unwrap_or_else(|| 128 + st.signal().unwrap_or(0));
    rec.end(code);
    Ok(code)
}

#[cfg(test)]
mod key_tests {
    use super::{Key, parse_keys};

    fn kinds(chunk: &[u8]) -> String {
        parse_keys(chunk)
            .iter()
            .map(|k| match k {
                Key::Char(c) => *c,
                Key::Enter => '\u{23ce}',
                Key::Backspace => '<',
                Key::KillLine => 'K',
                Key::Up => 'U',
                Key::Down => 'D',
                Key::Esc => 'E',
                Key::CtrlC => 'C',
            })
            .collect()
    }

    #[test]
    fn csi_and_ss3_arrows_both_parse() {
        // CSI form (tests, some terminals) and SS3 form (application
        // cursor mode — what zle-driven live sessions actually send).
        assert_eq!(kinds(b"\x1b[A\x1b[B"), "UD");
        assert_eq!(kinds(b"\x1bOA\x1bOB"), "UD");
    }

    #[test]
    fn parameterized_csi_never_leaks_into_filter() {
        assert_eq!(kinds(b"\x1b[1;3Bgem"), "gem");
    }

    #[test]
    fn lone_esc_and_controls() {
        assert_eq!(kinds(b"\x1b"), "E");
        assert_eq!(kinds(b"\x03\r\x7f\x15q"), "C\u{23ce}<Kq");
    }
}
