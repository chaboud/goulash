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
use std::time::{Duration, Instant};

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
    /// What was asked. `text` is the answer; the inset row stubs the
    /// *question*, because that is what says which turn you are looking
    /// at when a finding has taken the row over.
    question: String,
    /// A researched finding for this turn. Fast stays primary and this
    /// **fills in beneath it** — an addition to the record rather than a
    /// replacement, so the stack reads as a transcript of what happened
    /// instead of a bait-and-switch.
    /// (wiki: architecture/two-lane-engagement.md)
    alt: Option<Finding>,
}

/// What the slow lane came back with, hung off the turn it answers.
#[derive(Clone)]
struct Finding {
    cmd: Option<String>,
    text: String,
    /// Kept, not shown — the justification behind the one line, there
    /// for when the user asks about the suggestion rather than for
    /// filling the band.
    reasoning: String,
}

const SUG_HIST_CAP: usize = 50;

/// The slot stack flattened for browsing: one entry per pullable
/// command, so a turn with a researched alternative contributes two —
/// fast's first, the finding second.
///
/// Down therefore walks *into* the alternative before moving to the next
/// turn (depth-first), which keeps the whole stack reachable on the one
/// axis the user's fingers already know. Reaching it sideways with Right
/// is the other candidate; this is the mechanic that needs no new key.
/// (wiki: architecture/two-lane-engagement.md)
fn flat_slots(hist: &[SugTurn]) -> Vec<(usize, bool)> {
    let mut out = Vec::with_capacity(hist.len());
    for (i, t) in hist.iter().enumerate() {
        out.push((i, false));
        if t.alt.as_ref().and_then(|a| a.cmd.as_ref()).is_some() {
            out.push((i, true));
        }
    }
    out
}

/// The command a flattened slot pastes.
fn slot_cmd(hist: &[SugTurn], (i, is_alt): (usize, bool)) -> Option<String> {
    let t = hist.get(i)?;
    if is_alt {
        t.alt.as_ref()?.cmd.clone()
    } else {
        Some(t.cmd.clone())
    }
}

/// Rows of shell the menu will never take: below this the overlay is
/// eating the terminal rather than annotating it.
const MENU_MIN_INNER: u16 = 10;

/// Floor and ceiling for the insurance repaint (session.rs, idle path).
///
/// It rescues a band damaged by output we failed to parse, so it is
/// armed by output and not by a clock — and it decays, because a stream
/// that has not broken the band in thirty seconds is unlikely to start.
/// A fixed one-second cadence wrote to the terminal 3,600 times an hour
/// on a session doing nothing, scroll-region set and all, which is not
/// free however invisible it looks.
const INSURE_MIN: Duration = Duration::from_secs(1);
const INSURE_MAX: Duration = Duration::from_secs(30);

/// The shared menu primitive (wiki: interaction/settings-and-nav.md):
/// modal, type-to-filter (no per-item hotkeys), the list scrolling under
/// a fixed cursor inside the fixed goulash area — no winsize change.
/// Only ever opened by the user, by name (bare `#/model`); Esc and
/// Ctrl-C always close it.
#[derive(Clone, Copy, PartialEq)]
enum MenuKind {
    /// Enter binds the model and persists it.
    Model,
    /// The same list for the research lane — which may be a different
    /// server entirely, so it is filled from THAT server's inventory.
    SlowModel,
    /// Enter opens the slot for reading; Backspace/Delete arms it and a
    /// second one forgets it. Destructive actions in a modal list need a
    /// confirm keystroke, not a hair-trigger — and they do not belong on
    /// the key every other menu uses to say "yes, this one".
    Memory,
    /// Enter cycles the setting's value in place, applying it live AND
    /// persisting it — no config-file round trip.
    Settings,
    /// A browsable command reference; Enter does nothing.
    Help,
    /// Terminal-hackery toggles. Same cycle-on-Enter mechanic as
    /// Settings, kept separate because these are levers on how goulash
    /// talks to the emulator, not preferences.
    Debug,
    /// `#@` working context. Enter opens the pin in the reading pane;
    /// Backspace/Delete twice unpins — same keymap as Memory, for the
    /// same reason.
    Pins,
}

/// Live-tunable settings and the values Enter cycles through. Everything
/// here applies immediately and persists; anything that would need a
/// restart stays in the TOML where it cannot mislead.
const SETTINGS: &[(&str, &[&str])] = &[
    ("commentary", &["on", "off"]),
    ("slow", &["ingest", "volunteer", "manual", "off"]),
    ("thinking", &["off", "low", "medium", "high"]),
    ("memory", &["off", "on"]),
    ("max_tokens", &["2048", "4096", "8192", "16384"]),
    ("command_first", &["on", "off"]),
    ("platform", &["on", "off"]),
    ("stats", &["off", "on"]),
];

/// `#/debug`: the drawer for behaviours that change how goulash drives
/// the terminal. Anything here can be turned live so a field problem can
/// be bisected in place instead of by rebuilding.
const DEBUG_SETTINGS: &[(&str, &[&str])] = &[
    ("cursor_save", &["decsc", "absolute"]),
    ("idle_repaint", &["on", "off"]),
    ("wrap_guard", &["off", "on"]),
    ("divulge_tools", &["off", "on"]),
    ("divulge_path", &["off", "on"]),
];

const HELP_ITEMS: &[&str] = &[
    "#<question>            ask; the answer lands in the band",
    "##<question>           chat: follow-ups need no prefix",
    "\u{2193} / \u{2191}                  suggestions below the prompt, history above",
    "#/model                pick a model (\u{23ce} saves it as default)",
    "#/model NAME [save]    switch now; 'save' persists",
    "#/memory               browse slots; + new to write one",
    "#/memory on|off|limit  enable, disable, resize the store",
    "#@/path FILE           pin a file (or dir) into the model's context",
    "#@ <request>           pin/unpin in words; #@ alone browses",
    "#@/unset               drop every pin",
    "#@/list \u{b7} #@/cancel    what's pinned \u{b7} stop a running ingest",
    "#/settings             live-tune everything below",
    "#? <question>          ask the slow lane; fast still answers first",
    "#?/cancel \u{b7} #/cancel   stop research \u{b7} stop everything",
    "#/debug                terminal-hackery toggles (esoteric)",
    "#/thinking off|low|medium|high",
    "#/commentary on|off    per-turn heckling",
    "#/status               engine, model, blocks this session",
];

/// One item opened for reading. Takes every row the terminal can spare
/// above `MENU_MIN_INNER`, which only became a sane thing to do once the
/// band stopped scribbling on the screen it grew into.
///
/// `top` indexes LOGICAL lines, not rendered rows: wrapping stays a
/// render-time concern, so a resize under an open pane re-flows the text
/// without moving the reader's place in it.
struct Viewer {
    title: String,
    lines: Vec<String>,
    top: usize,
}

struct Menu {
    title: String,
    kind: MenuKind,
    items: Vec<String>,
    filter: String,
    cursor: usize, // index into the filtered view
    loaded: bool,
    armed: Option<String>,
    /// Typing a new entry from inside the list (the `+ new` row).
    composing: Option<String>,
    /// Reading one item full-height. Modal within the menu: Esc backs
    /// out to the list, not out of the menu.
    viewing: Option<Viewer>,
}

impl Menu {
    fn open(title: &str, kind: MenuKind) -> Menu {
        Menu {
            title: title.to_string(),
            kind,
            items: Vec::new(),
            filter: String::new(),
            cursor: 0,
            loaded: kind == MenuKind::Memory,
            armed: None,
            composing: None,
            viewing: None,
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
    /// Forward delete (`ESC[3~`). On a Mac laptop the key *labelled*
    /// Delete sends Backspace, so anything destructive has to answer to
    /// both or it answers to neither.
    Delete,
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
                                b'~' if &chunk[i + 2..j] == b"3" => keys.push(Key::Delete),
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

/// The chrome-row diagnostics line, or None when the setting is off.
/// Refreshes the live counts here rather than at a dozen mutation sites:
/// they are all cheap `len()` calls, and one place that cannot drift
/// beats twelve that can.
#[allow(clippy::too_many_arguments)]
fn stats_line(
    on: bool,
    stats: &mut crate::stats::Stats,
    sug_hist: &[SugTurn],
    held: usize,
    work: &crate::context::WorkContext,
    ctx_log: &str,
    num_ctx: usize,
) -> Option<String> {
    if !on {
        return None;
    }
    stats.slots = sug_hist.len();
    stats.held = held;
    stats.pins = work.list().len();
    stats.ctx_chars = ctx_log.len();
    stats.num_ctx = num_ctx;
    if let Some(dir) = Config::dir() {
        stats.sample(&dir);
    }
    Some(stats.line())
}

/// Everything that rides next to the question: the memories most
/// relevant to it, then the pinned files' cards.
///
/// One helper because they are one position, and the position is the
/// whole point — the stable prefix already holds complete copies of
/// both, cache-warm and utterly ignorable by a sliding-window model.
/// Memories were left out of that reasoning when the pin cards were
/// built; a slot saying macOS `du` wants `-d <depth>` sat in the prefix
/// while the model suggested `--max-depth=1` twice running.
fn near_question(memory: &MemoryStore, work: &crate::context::WorkContext, q: &str) -> String {
    format!("{}{}", memory.cards_block(q), work.cards_block())
}

fn hist_push(hist: &mut Vec<SugTurn>, turn: SugTurn) {
    if let Some(top) = hist.first_mut()
        && top.cmd == turn.cmd
    {
        // Adjacent dedup: re-vending the same fix is not a new turn, so
        // it gets no new slot. But the ID has to move to it, because
        // research for THIS ask was dispatched against the new id and
        // `apply_finding` drops a finding whose turn it cannot find —
        // so an explicit `#?` on a repeated question silently produced
        // nothing at all.
        top.id = turn.id;
        return;
    }
    hist.insert(0, turn);
    hist.truncate(SUG_HIST_CAP);
}

/// Hard-wrap ONE logical line, keeping its interior whitespace (a file
/// being read has meaningful indentation). Always yields at least one
/// part, including for an empty line — a caller counting rows against a
/// pane height would otherwise spin forever on a blank.
fn wrap_hard(line: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    // Control bytes from a pinned file would drive the emulator, not
    // print. Tabs become spaces so the width count stays honest.
    let line: String = line
        .replace('\t', "    ")
        .chars()
        .map(|c| if (c as u32) < 0x20 { ' ' } else { c })
        .collect();
    if line.chars().count() <= width {
        return vec![line];
    }
    let mut parts = Vec::new();
    let mut cur = String::new();
    for ch in line.chars() {
        cur.push(ch);
        if cur.chars().count() == width {
            parts.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    parts
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
    pin: Option<&str>,
    stats: Option<&str>,
    dots: &str,
) -> Vec<String> {
    // Never write the terminal's LAST cell. A row that fills the final
    // column is flagged as continued/soft-wrapped, and a width change
    // makes the emulator reflow it into a second line — field-observed
    // on macOS Terminal as trailing rule fragments sprayed up the
    // scrollback during a drag-resize.
    let cols = (layout.real.cols as usize).saturating_sub(1);
    let reserved_now = cfg.reserved_rows();
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
        rows.push(status::rule_row(
            Some(" ## chat "),
            Some(status::SUGGEST_SGR),
            Some(tip),
            cols,
        ));
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
            pin,
            stats,
            dots,
        ));
        return rows;
    }
    if cfg.status.band
        && let Some(m) = menu
    {
        // Modal menu: the list still scrolls under a fixed cursor, but
        // the area GROWS to hold a usable number of rows when the
        // terminal can spare them — two item rows is not a browser.
        // User-initiated, temporary, and it gives the rows straight
        // back on close, so the no-surprise-resize rule is intact.
        let base = cfg.status.band_rows.clamp(1, 4) + 1;
        // Never squeeze the shell below MENU_MIN_INNER rows: the inner
        // world matters more than our list.
        let room = layout
            .real
            .rows
            .saturating_sub(reserved_now + MENU_MIN_INNER);
        // Reading a file wants every row going; browsing a list does
        // not, so the pane ignores menu_rows and takes the lot — held to
        // the same MENU_MIN_INNER floor, counted against what `reserved`
        // will actually become (items + rule + chrome).
        let want = match m.viewing {
            Some(_) => layout.real.rows.saturating_sub(MENU_MIN_INNER + 2),
            None => cfg.status.menu_rows.clamp(2, 20),
        };
        let n_items = want.min(base.saturating_add(room)).max(base) as usize;
        let reserved = n_items as u16 + 2; // rule + items + chrome
        let inner = layout.real.rows.saturating_sub(reserved).max(1);
        if let Some(v) = &m.viewing {
            let n = v.lines.len();
            let top = v.top.min(n.saturating_sub(1));
            let mut rows = Vec::new();
            rows.push(status::rule_row(
                Some(&format!(" {} ", v.title)),
                Some(status::SUGGEST_SGR),
                Some(&format!(
                    " \u{2191}\u{2193} scroll \u{b7} esc back \u{b7} {}/{} ",
                    (top + 1).min(n),
                    n
                )),
                cols,
            ));
            // Wrapping happens HERE, not at open time: cols is a render
            // fact, and `top` counts logical lines, so a resize reflows
            // the text without moving the reader's place in it.
            let mut painted = 0usize;
            let mut idx = top;
            while painted < n_items {
                match v.lines.get(idx) {
                    Some(line) => {
                        for part in wrap_hard(line, cols.saturating_sub(1)) {
                            if painted == n_items {
                                break;
                            }
                            rows.push(status::pad_row(&format!(" {part}"), cols, status::TEXT_SGR));
                            painted += 1;
                        }
                        idx += 1;
                    }
                    None => {
                        rows.push(status::pad_row("", cols, status::TEXT_SGR));
                        painted += 1;
                    }
                }
            }
            rows.push(status::chrome_row(
                layout.real,
                inner,
                reserved,
                shell_name,
                sense::label(st, hook),
                pin,
                stats,
                dots,
            ));
            return rows;
        }
        let filtered = m.filtered();
        let chip = match &m.composing {
            Some(text) => format!(" {} \u{25b8} + {}\u{258f} ", m.title, text),
            None => format!(" {} \u{25b8} {}\u{258f} ", m.title, m.filter),
        };
        // Feedback for in-menu actions has nowhere else to go: the rule
        // row belongs to the menu while it is open, so a notice takes
        // the keymap's place until the next keystroke.
        let tip = match notice {
            Some(n) => format!(" {n} "),
            None if m.composing.is_some() => " \u{23ce} save \u{b7} esc cancel ".to_string(),
            None => format!(
                " \u{2191}\u{2193} \u{b7} {} \u{b7} esc \u{b7} {}/{} ",
                match m.kind {
                    MenuKind::Model | MenuKind::SlowModel => "\u{23ce} save",
                    MenuKind::Memory => "\u{23ce} read \u{b7} \u{232b}\u{232b} forget",
                    MenuKind::Pins => "\u{23ce} read \u{b7} \u{232b}\u{232b} unpin",
                    MenuKind::Settings | MenuKind::Debug => "\u{23ce} cycles",
                    MenuKind::Help => "reference",
                },
                (m.cursor + 1).min(filtered.len()),
                filtered.len()
            ),
        };
        let mut rows = Vec::new();
        rows.push(status::rule_row(
            Some(&chip),
            Some(status::SUGGEST_SGR),
            Some(&tip),
            cols,
        ));
        // Cursor pinned near the bottom of the window; list slides.
        let top = (m.cursor + 1).saturating_sub(n_items);
        for row in 0..n_items {
            let idx = top + row;
            let line = match filtered.get(idx) {
                // The armed prompt goes at the FRONT. A pin row is a
                // full path and blows past 80 columns routinely, so a
                // suffix tag is exactly the part that clips — and the
                // part that clips must never be the one warning you
                // that the next keystroke destroys something.
                Some(name) if m.armed.as_deref() == Some(*name) => {
                    let verb = match m.kind {
                        MenuKind::Pins => "unpin",
                        _ => "forget",
                    };
                    format!(" \u{232b} again to {verb} \u{2192} {name}")
                }
                Some(name) => {
                    let tag = if m.kind == MenuKind::Model && Some(*name) == engine_model.as_deref()
                    {
                        "*"
                    } else {
                        ""
                    };
                    format!(" {name}{tag}")
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
            inner,
            reserved,
            shell_name,
            sense::label(st, hook),
            pin,
            stats,
            dots,
        ));
        return rows;
    }
    // While browsing the slot history, the browsed turn owns the area:
    // its command in the chip, its chat text in the band, position on
    // the rule's right end. Everything else is frozen underneath.
    // Browsing walks the FLATTENED stack, so resolve the position back
    // to (turn, is-it-the-alternative).
    let flat = flat_slots(sug_hist);
    let here = browse.and_then(|p| flat.get(p).copied());
    let browsed = here.and_then(|(i, alt)| sug_hist.get(i).map(|t| (i, t, alt)));
    // The chip always shows FAST's command; the finding has its own row
    // below. Which of the two is orange says which one Enter pulls.
    let sug_chip = match browsed {
        Some((_, t, _)) => Some(format!(" \u{2193} suggestion: {} ", t.cmd)),
        None => suggestions
            .first()
            .map(|s| format!(" \u{2193} suggestion: {} ", s.1)),
    };
    let alt_selected = browsed.map(|(_, _, is_alt)| is_alt).unwrap_or(false);
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
            pin,
            stats,
            dots,
        )];
    }

    let n_text = cfg.status.band_rows.clamp(1, 4);
    let mut rows = Vec::new();
    // Right end of the rule: scroll position while browsing the slot
    // history; otherwise the ingress tip — until a pullable suggestion
    // exists (the command is the more important thing).
    let tip = match browse.filter(|_| browsed.is_some()) {
        Some(p) => Some(format!(
            " \u{2191} {}/{}{} ",
            p + 1,
            flat.len(),
            if p + 1 < flat.len() { " \u{2193}" } else { "" }
        )),
        None if sug_chip.is_none() => {
            Some(" # message to chat \u{b7} #/help for help ".to_string())
        }
        None => None,
    };
    rows.push(status::rule_row(
        rule_text.as_deref(),
        sug_chip.as_ref().map(|_| {
            if alt_selected {
                status::IDLE_CHIP_SGR
            } else {
                status::SUGGEST_SGR
            }
        }),
        tip.as_deref(),
        cols,
    ));
    // The question row doubles as the slot for a researched finding.
    // When one exists for the browsed turn it overlays here, indented
    // and in its own colour — the question was not doing much work on
    // this row anyway, so it truncates to a stub and an ellipsis.
    let alt = browsed.and_then(|(_, t, _)| t.alt.as_ref());
    match alt {
        Some(a) => {
            let stub = match browsed.map(|(_, t, _)| t.question.as_str()) {
                Some(q) if !q.is_empty() => {
                    format!(" {}\u{2026} ", q.chars().take(20).collect::<String>())
                }
                _ => " ".to_string(),
            };
            let cmd = a.cmd.as_deref().unwrap_or(&a.text);
            rows.push(status::inset_row(
                &stub,
                &format!("\u{21b3} {cmd}"),
                cols,
                alt_selected,
            ));
        }
        None => {
            let q = match browsed {
                Some(_) => "suggestion history",
                None => band
                    .as_ref()
                    .and_then(|b| b.question.as_deref())
                    .unwrap_or(""),
            };
            rows.push(status::pad_row(&format!(" {q}"), cols, status::QUERY_SGR));
        }
    }
    let mut lines = match browsed {
        // A finding's one line explains the inset above it; the turn's
        // own text is what the question row already stubbed.
        Some((_, t, _)) if t.alt.is_some() => wrap_chars(
            &t.alt.as_ref().unwrap().text,
            cols.saturating_sub(2),
            n_text as usize,
        ),
        Some((_, t, _)) => wrap_chars(&t.text, cols.saturating_sub(2), n_text as usize),
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
        pin,
        stats,
        dots,
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
/// restore.
///
/// The restore is the subtle part. We interrupt a line editor that
/// believes it owns the cursor, so whatever we put back has to be
/// **exactly** what was there — including the state no escape sequence
/// can name. After a glyph lands in the terminal's last column the
/// cursor is in *deferred wrap*: it reads as still on that row, but the
/// next glyph moves to the next line. `CUP` cannot express that, so
/// restoring by absolute position silently cancels it, the shell's next
/// character overwrites the last cell instead of wrapping, and every row
/// below shifts by one while the editor's own line accounting does not.
/// Field signature: a tab-completion listing that clears one row short.
///
/// `ESC 7` / `ESC 8` (DECSC/DECRC) is the terminal's own save/restore and
/// carries the wrap flag with it. Its one cost is that the emulator has a
/// single save slot, shared with the child — so a child that saves,
/// gets painted over, then restores would get our cursor back. Line
/// editors do not use DECSC, and full-screen apps that do are painted
/// over only in the alt screen, where the band is suspended anyway.
/// `[debug] cursor_save = "absolute"` reverts to the old behaviour.
///
/// DECSC does not cover cursor *visibility*, so that is re-asserted from
/// the mirror afterwards either way.
fn fixup_bytes(
    layout: &Layout,
    screen: &vt100::Screen,
    rows: &[String],
    cursor_save: &str,
) -> Vec<u8> {
    let decsc = cursor_save != "absolute";
    let mut out: Vec<u8> = Vec::with_capacity(512);
    let inner = layout.inner();
    if decsc {
        out.extend_from_slice(b"\x1b7"); // DECSC, before we disturb anything
    }
    out.extend_from_slice(b"\x1b[?25l"); // hide cursor while we work
    out.extend_from_slice(format!("\x1b[1;{}r", inner.rows).as_bytes());
    for (i, row) in rows.iter().enumerate() {
        out.extend_from_slice(format!("\x1b[{};1H", inner.rows + 1 + i as u16).as_bytes());
        // Reset+erase first: the row payload stops one cell short of the
        // right edge (see compose_rows), and this guarantees that last
        // cell is *erased* rather than merely unwritten.
        out.extend_from_slice(b"\x1b[0m\x1b[K");
        out.extend_from_slice(row.as_bytes());
    }
    if decsc {
        out.extend_from_slice(b"\x1b8"); // DECRC: position, attrs, wrap flag
    } else {
        out.extend_from_slice(&screen.attributes_formatted());
        out.extend_from_slice(&screen.cursor_state_formatted());
    }
    out.extend_from_slice(if screen.hide_cursor() {
        b"\x1b[?25l".as_slice()
    } else {
        b"\x1b[?25h".as_slice()
    });
    out
}

/// Is the inner cursor parked in the last column — i.e. is the terminal
/// (probably) holding a deferred wrap right now? The mirror tracks the
/// column but not the flag itself, so this is the closest proxy we have,
/// and it is only ever used to *defer* a paint, never to change one.
fn at_last_column(screen: &vt100::Screen, layout: &Layout) -> bool {
    let (_, col) = screen.cursor_position();
    col + 1 >= layout.inner().cols
}

/// Hand back the rows the band occupied last paint but no longer does.
///
/// A resize gives those rows to the inner world, and the inner world
/// only rebuilds itself at the next **prompt turn** — so blank-erasing
/// them would punch a hole that nothing repairs until the user presses
/// Enter. Each row is instead repainted from the vt100 mirror, which is
/// the shell's own truth about what belongs there: our stale paint goes
/// away and the shell's content (blank or not) comes back.
fn reclaim_rows(
    last: (u16, u16),
    top: u16,
    height: u16,
    layout: &Layout,
    screen: &vt100::Screen,
) -> Vec<u8> {
    let (old_top, old_height) = last;
    let mut out = Vec::new();
    if old_height == 0 || (old_top, old_height) == (top, height) {
        return out;
    }
    let inner_rows = layout.inner().rows;
    let mut mirror: Option<Vec<Vec<u8>>> = None;
    for r in old_top..old_top.saturating_add(old_height) {
        if r >= top && r < top.saturating_add(height) {
            continue; // still ours — the band repaint covers it
        }
        if r < 1 || r > layout.real.rows {
            continue; // scrolled off the screen entirely
        }
        out.extend_from_slice(format!("\x1b[{r};1H\x1b[0m\x1b[K").as_bytes());
        if r <= inner_rows {
            let rows =
                mirror.get_or_insert_with(|| screen.rows_formatted(0, layout.real.cols).collect());
            if let Some(content) = rows.get((r - 1) as usize) {
                out.extend_from_slice(content);
            }
        }
    }
    out
}

/// `#/` command dispatch over the full command line (memory verbs take
/// free text). Returns the bar notice to show.
#[allow(clippy::too_many_arguments)]
fn slash_command(
    cmdline: &str,
    engine: Option<&Engine>,
    blocks: u64,
    commentary: &mut bool,
    memory: &mut MemoryStore,
    fuse: &mut StateFile,
    menu: &mut Option<Menu>,
    thinking: &mut String,
    max_tokens: usize,
    command_first: bool,
    stats: bool,
    caps: Option<&crate::models::Caps>,
    dbg: &crate::config::DebugConfig,
    slow: &str,
    platform: bool,
) -> Option<String> {
    let mut it = cmdline.splitn(2, char::is_whitespace);
    let cmd = it.next().unwrap_or("");
    let rest = it.next().unwrap_or("").trim();
    let arg = if rest.is_empty() { None } else { Some(rest) };
    match (cmd, arg) {
        ("memory", None) | ("memory", Some("list")) => {
            // Browsing 50 slots through one bar row is unusable; the
            // menu primitive already solves this.
            let title = if memory.enabled {
                "memory"
            } else {
                "memory (off)"
            };
            let mut m = Menu::open(title, MenuKind::Memory);
            m.items = memory_items(memory);
            *menu = Some(m);
            None
        }
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
                *menu = Some(Menu::open("model", MenuKind::Model));
                None
            }
            None => Some("no engine running".to_string()),
        },
        ("thinking", arg) => match engine {
            Some(eng) => {
                let level = arg
                    .map(|a| a.split_whitespace().next().unwrap_or(a))
                    .unwrap_or("off");
                match level {
                    "off" | "on" | "low" | "medium" | "high" => {
                        eng.set_option("thinking", level);
                        *thinking = level.to_string();
                        let _ = Config::persist_key("engine", "thinking", level);
                        // Setting it is honest; pretending it lands is
                        // not. Say plainly when the model won't oblige.
                        Some(format!("thinking {level}{}", thinking_note(caps)))
                    }
                    _ => Some("usage: #/thinking off|low|medium|high".to_string()),
                }
            }
            None => Some("no engine running".to_string()),
        },
        ("settings", _) | ("config", _) => {
            let mut m = Menu::open("settings", MenuKind::Settings);
            m.items = settings_items(&Live {
                platform,
                commentary: *commentary,
                slow,
                thinking,
                max_tokens,
                command_first,
                stats,
                memory,
                caps,
            });
            *menu = Some(m);
            None
        }
        ("debug", _) => {
            let mut m = Menu::open("debug", MenuKind::Debug);
            m.items = debug_items(dbg);
            m.loaded = true;
            *menu = Some(m);
            None
        }
        ("help", _) => {
            let mut m = Menu::open("help", MenuKind::Help);
            m.items = HELP_ITEMS.iter().map(|s| s.to_string()).collect();
            m.loaded = true;
            *menu = Some(m);
            None
        }
        ("cancel", _) => match engine {
            Some(eng) => {
                // The sigil scopes the cancel: this is the bare one, so
                // it stops everything goulash has in flight.
                eng.cancel_research();
                eng.cancel_digests();
                Some("cancelled background work".to_string())
            }
            None => Some("no engine running".to_string()),
        },
        ("status", _) => {
            // The lane picture comes back asynchronously from the
            // worker, which is the only place that knows what actually
            // bound where; this line is what the user has right now.
            if let Some(eng) = engine {
                eng.describe_lanes();
            }
            // Deliberately short: the lane line that follows carries the
            // model, the provider, and what each is trusted with, and
            // the two together have to fit one bar row at 80 columns.
            Some(match engine {
                Some(_) => format!(
                    "goulash {} \u{b7} {blocks} blocks",
                    env!("CARGO_PKG_VERSION")
                ),
                None => format!(
                    "goulash {} \u{b7} no engine \u{b7} {blocks} blocks",
                    env!("CARGO_PKG_VERSION")
                ),
            })
        }
        _ => Some(format!("unknown command /{cmd} \u{2014} try #/help")),
    }
}

/// `name: value` rows for the settings menu, from live state. The
/// thinking row is annotated with what the bound model will actually do
/// with it — a dial that silently does nothing is worse than no dial.
/// The live value of every `#/settings` row, in one place.
///
/// It was a growing list of positional scalars, and that is precisely
/// how `command_first` ended up displayed from a hardcoded literal while
/// the real value lived elsewhere: nothing forced the two to meet. A
/// struct makes adding a setting a compile error at every site that has
/// to know about it.
struct Live<'a> {
    commentary: bool,
    platform: bool,
    slow: &'a str,
    thinking: &'a str,
    max_tokens: usize,
    command_first: bool,
    stats: bool,
    memory: &'a MemoryStore,
    caps: Option<&'a crate::models::Caps>,
}

fn settings_items(v: &Live) -> Vec<String> {
    let (commentary, slow, thinking, max_tokens, command_first, stats, memory, caps) = (
        v.commentary,
        v.slow,
        v.thinking,
        v.max_tokens,
        v.command_first,
        v.stats,
        v.memory,
        v.caps,
    );
    SETTINGS
        .iter()
        .map(|(name, _)| {
            let v = match *name {
                "commentary" => if commentary { "on" } else { "off" }.to_string(),
                "slow" => slow.to_string(),
                "thinking" => format!("{thinking}{}", thinking_note(caps)),
                "memory" => if memory.enabled { "on" } else { "off" }.to_string(),
                "max_tokens" => max_tokens.to_string(),
                "command_first" => if command_first { "on" } else { "off" }.to_string(),
                "stats" => if stats { "on" } else { "off" }.to_string(),
                _ => String::new(),
            };
            format!("{name}: {v}")
        })
        .collect()
}

/// `#@` — the working context surface.
///
/// Two dialects on purpose. The `/` forms are a pure, deterministic
/// path API: no model involved, so they work with no engine bound, they
/// are exactly testable, and — because a `#@/path …` line is an ordinary
/// shell comment goulash intercepts — the model can *suggest* one as a
/// `CMD:` line that the user pulls with Down like any other. Everything
/// else is handed to the model, which answers in PIN verbs (context.rs).
#[allow(clippy::too_many_arguments)]
fn at_command(
    rest: &str,
    work: &mut crate::context::WorkContext,
    engine: Option<&Engine>,
    cwd: &str,
    bound: bool,
    menu: &mut Option<Menu>,
) -> Option<String> {
    let rest = rest.trim();
    // Paths are the USER's, so they resolve against the shell's cwd —
    // which goulash learns from the OSC wire, not from its own process.
    // goulash was launched wherever it was launched; the shell has been
    // cd-ing around ever since.
    let base = std::path::Path::new(if cwd.is_empty() { "." } else { cwd });
    let resolve = |p: &str| base.join(shellexpand(p));
    if let Some(sub) = rest.strip_prefix('/') {
        let mut it = sub.splitn(2, char::is_whitespace);
        let verb = it.next().unwrap_or("");
        let arg = it.next().unwrap_or("").trim();
        return Some(match (verb, arg) {
            // A blank path is the unset: `#@/path ` with nothing after
            // it reads as "stop anchoring on anything".
            ("path", "") | ("unset", _) | ("clear", _) => {
                if let Some(eng) = engine {
                    eng.cancel_digests();
                }
                work.cancel_cooking();
                match work.clear() {
                    0 => "@ nothing pinned".to_string(),
                    n => format!("@ cleared ({n})"),
                }
            }
            ("path", p) => {
                let out = match work.pin(&resolve(p)) {
                    Ok(msg) => msg,
                    Err(e) => format!("@ {e}"),
                };
                kick_digests(work, engine, bound);
                out
            }
            ("drop", a) => match a
                .trim_start_matches('[')
                .trim_end_matches(']')
                .parse::<u64>()
            {
                Ok(id) => match work.drop_id(id) {
                    Some(label) => {
                        // One fewer pin means a bigger share for the
                        // rest; some of them may now fit unaided.
                        kick_digests(work, engine, bound);
                        format!("@ dropped {label}")
                    }
                    None => format!("@ no pin [{id}]"),
                },
                Err(_) => "usage: #@/drop <id>".to_string(),
            },
            // A long cook has to be abandonable — that was the whole
            // point of making ingest asynchronous.
            ("cancel", _) => {
                if let Some(eng) = engine {
                    eng.cancel_digests();
                }
                match work.cancel_cooking() {
                    0 => "@ nothing cooking".to_string(),
                    n => format!("@ cancelled {n}"),
                }
            }
            ("list", _) => list_pins(work),
            _ => "usage: #@/path <file> \u{b7} #@/unset \u{b7} #@/drop <id> \
                  \u{b7} #@/list \u{b7} #@/cancel"
                .to_string(),
        });
    }
    if rest.is_empty() {
        // A notice listing 50 slots is unreadable — the exact argument
        // that turned #/memory into a menu. Same primitive here, and it
        // is what makes machine-written artifacts safe to have: goulash
        // decides what to build, the user can always see it and bin it.
        let mut m = Menu::open("@ pinned", MenuKind::Pins);
        m.items = pin_items(work);
        m.loaded = true;
        *menu = Some(m);
        return None;
    }
    // A literal path resolves DIRECTLY — no model call, no latency, no
    // chance of a mis-resolve. `#@ .` and `#@ ./ref.md` are paths the
    // user typed, not requests to interpret, and sending them to a model
    // was both slower and worse.
    let literal = resolve(rest);
    if literal.exists() {
        let out = match work.pin(&literal) {
            Ok(msg) => msg,
            Err(e) => format!("@ {e}"),
        };
        kick_digests(work, engine, bound);
        return Some(out);
    }

    // Anything that is not a path is handed to the model, which resolves
    // it against a listing of candidates and answers in PIN verbs.
    // Read-only, and goulash does the reading — a mis-resolved pin costs
    // a wasted read, not a side effect, which is why this needs no
    // approval prompt in front of it.
    match engine {
        Some(eng) => {
            eng.ask_pin(
                rest.to_string(),
                crate::context::WorkContext::candidates(base),
                work.context_block(),
            );
            None
        }
        None => Some("no engine running \u{2014} try #@/path <file>".to_string()),
    }
}

/// What the chrome chip carries about goulash's own state: the active
/// pin, and a `?` while the slow lane is working.
///
/// A silent multi-second research job is the same "am I frozen?" failure
/// as a silent model load, and the answer is the same — say so in the
/// one place that is always on screen.
fn chrome_tag(work: &crate::context::WorkContext, researching: Option<u64>) -> Option<String> {
    let pin = work.chrome_tag();
    match (pin, researching.is_some()) {
        (Some(p), true) => Some(format!("{p} \u{b7} ?\u{2026}")),
        (Some(p), false) => Some(p),
        (None, true) => Some("?\u{2026}".to_string()),
        (None, false) => None,
    }
}

/// Dispatch a `#?` / `?` question to the slow lane, returning the notice
/// (if any) to show.
///
/// With slow unavailable this **answers via fast and says so** rather
/// than refusing: the user asked a question and deserves an answer, and
/// silently downgrading would be worse than either.
#[allow(clippy::too_many_arguments)]
fn ask_slow(
    body: &str,
    turn: u64,
    engine: Option<&Engine>,
    slow: &str,
    ctx_log: &str,
    memories: String,
    pinned: String,
    cards: String,
) -> Option<String> {
    let Some(eng) = engine else {
        return Some("no engine running".to_string());
    };
    if slow == "off" {
        eng.ask(
            body.to_string(),
            ctx_log.to_string(),
            memories,
            pinned,
            cards,
        );
        // Said on the question row, not as a notice: a notice is
        // transient and the answer arriving would wipe it, so the user
        // would never learn why their `#?` behaved like a `#`.
        return Some("answered by fast \u{b7} #/settings \u{2192} slow".to_string());
    }
    // Fast answers first and keeps the microphone; slow researches the
    // same turn and amends it if it finds something better.
    eng.ask(
        body.to_string(),
        ctx_log.to_string(),
        memories.clone(),
        pinned.clone(),
        cards,
    );
    eng.research(
        turn,
        body.to_string(),
        ctx_log.to_string(),
        memories,
        pinned,
    );
    None
}

/// Land a finding on the turn it belongs to, and record it in the
/// session log **by reference** rather than by rewriting what was
/// already written.
///
/// The log is fast's own memory of the conversation. Silently replacing
/// an earlier `CMD:` would leave that memory disagreeing with what the
/// user is looking at — harder for a small model to follow than an
/// overwrite would be, and survivable, which divergence is not.
fn apply_finding(hist: &mut [SugTurn], turn: u64, finding: Finding, ctx_log: &mut String) {
    let Some(slot) = hist.iter_mut().find(|t| t.id == turn) else {
        return; // the turn aged out of the stack; nothing to amend
    };
    if let Some(cmd) = &finding.cmd {
        ctx_log.push_str(&format!("CMD: {cmd} [amends the suggestion above]\n"));
    }
    // The reasoning is retained rather than shown — but retained *where
    // fast can read it*, because fast is the one who will be asked
    // "why?" and it is the only voice. Bounded: this is a receipt, not a
    // second transcript competing for the prompt.
    if !finding.reasoning.is_empty() {
        let brief: String = finding
            .reasoning
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(400)
            .collect();
        ctx_log.push_str(&format!("[researched: {brief}]\n"));
    }
    slot.alt = Some(finding);
}

/// Ask the engine to compress any pin that no longer fits its share.
/// Called after anything that changes the pin set — including *removing*
/// one, since a smaller pin list means a bigger share each and a digest
/// that was too big may now fit.
///
/// The deterministic outline is already serving in the meantime, so this
/// is pure upside: if the engine is absent, slow, or refuses, nothing
/// waits and nothing breaks.
fn kick_digests(work: &mut crate::context::WorkContext, engine: Option<&Engine>, bound: bool) {
    // Attempts are finite, so don't spend one on a worker with no model
    // bound yet — ollama may simply have started after we did, and the
    // next pin (or the next attempt trigger) will find it.
    let Some(eng) = engine.filter(|_| bound) else {
        return;
    };
    for (id, label, source, target) in work.digest_wanted() {
        eng.digest(id, label, source, target, false);
    }
    // Every pin wants a card, not only the oversized ones: even a small
    // file benefits from having its key lines restated where a
    // sliding-window model will actually attend to them.
    for (id, label, source, target) in work.card_wanted() {
        eng.digest(id, label, source, target, true);
    }
}

/// `~` is the one expansion a comment line never gets from the shell,
/// and the one a user will absolutely type. Nothing else is expanded —
/// this is a path, not a command line.
fn shellexpand(p: &str) -> String {
    match p.strip_prefix("~/") {
        Some(rest) => match std::env::var_os("HOME") {
            Some(h) => format!("{}/{rest}", h.to_string_lossy()),
            None => p.to_string(),
        },
        None => p.to_string(),
    }
}

fn list_pins(work: &crate::context::WorkContext) -> String {
    let pins = work.list();
    if pins.is_empty() {
        "@ nothing pinned \u{2014} '#@/path <file>' to anchor on one".to_string()
    } else {
        pins.join("  \u{b7}  ")
    }
}

/// The browser's first row: pinning should not require remembering the
/// `#@/path` incantation, exactly as `+ new memory` does for slots.
/// `"[7] /path/to/thing \u{b7} …"` -> `7`. Menu rows carry their store's
/// id in the text, so the list and the store need no parallel index.
fn item_id(item: &str) -> Option<u64> {
    item.trim_start_matches('[')
        .split(']')
        .next()
        .and_then(|s| s.parse::<u64>().ok())
}

const NEW_PIN: &str = "+ pin a file \u{2026}";

/// Rows for the pin browser, after the `+ pin` row.
fn pin_items(work: &crate::context::WorkContext) -> Vec<String> {
    std::iter::once(NEW_PIN.to_string())
        .chain(work.list())
        .collect()
}

/// `name: value` rows for the debug menu, from the live knobs.
fn debug_items(dbg: &crate::config::DebugConfig) -> Vec<String> {
    DEBUG_SETTINGS
        .iter()
        .map(|(name, _)| {
            let v = match *name {
                "cursor_save" => dbg.cursor_save.clone(),
                "idle_repaint" => if dbg.idle_repaint { "on" } else { "off" }.to_string(),
                "wrap_guard" => if dbg.wrap_guard { "on" } else { "off" }.to_string(),
                _ => String::new(),
            };
            format!("{name}: {v}")
        })
        .collect()
}

/// The parenthetical after the thinking value: silent when the model
/// honours the dial, loud when it cannot or when goulash is guessing.
fn thinking_note(caps: Option<&crate::models::Caps>) -> String {
    use crate::models::{Source, Think};
    let Some(c) = caps else {
        return String::new();
    };
    match (c.think, c.source) {
        (Think::None, _) => "  (no effect \u{2014} this model doesn't reason)".to_string(),
        (_, Source::Guess) => "  (unverified for this model)".to_string(),
        _ if c.always_reasons => "  (this model reasons regardless)".to_string(),
        _ => String::new(),
    }
}

/// The browser's first row: writing a memory should not require
/// remembering the `#/memory add` incantation.
const NEW_MEMORY: &str = "+ new memory \u{2026}";

/// Slot lines for the memory browser: `[id] text`, after the `+ new` row.
fn memory_items(memory: &MemoryStore) -> Vec<String> {
    std::iter::once(NEW_MEMORY.to_string())
        .chain(
            memory
                .find("")
                .iter()
                .map(|s| format!("[{}] {}", s.id, s.text)),
        )
        .collect()
}

/// `#/memory on|off|limit N|add TEXT|delete ID|modify ID TEXT|find Q`
fn memory_command(sub: Option<&str>, memory: &mut MemoryStore) -> String {
    let sub = sub.unwrap_or("");
    let mut it = sub.splitn(2, char::is_whitespace);
    let verb = it.next().unwrap_or("");
    let rest = it.next().unwrap_or("").trim();
    match verb {
        "" | "status" => memory.status_line(),
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
    // Startup: scroll the visible screen into scrollback, then begin
    // clean at the top.
    //
    // Starting *in place* was the obvious thing and it was wrong. Setting
    // the scroll region homes the cursor, the shell's first prompt cycle
    // then draws from the region top, and the new session paints
    // downward over whatever was there — while everything below the
    // write head keeps the old content. Nothing scrolls, so nothing
    // reaches scrollback: the visible screen is not preserved, it is
    // half-overwritten. Field-reported as "a fresh prompt at the top,
    // trash in the middle, and no scrollback", and reproduced by
    // launching after an `ls -R`.
    //
    // Newlines at the bottom margin SCROLL, so `row` of them push every
    // used line up and off. That is the whole difference from a clear:
    // the screen ends up equally blank, and every line of it is one
    // scroll-wheel away instead of destroyed.
    let (row, _col, typed_ahead) = query_cursor_row(layout.real);
    let inner_rows = layout.inner().rows;
    let mut init: Vec<u8> = Vec::new();
    init.extend_from_slice(format!("\x1b[{};1H", layout.real.rows).as_bytes());
    init.extend(std::iter::repeat_n(b'\n', row as usize));
    init.extend_from_slice(format!("\x1b[1;{inner_rows}r").as_bytes());
    init.extend_from_slice(b"\x1b[1;1H");
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
        Engine::start(eng_cfg, cfg.models.clone()).ok()
    } else {
        None
    };
    let mut engine_model: Option<String> = None;
    let mut commentary = cfg.engine.commentary;
    let mut memory = MemoryStore::load(Config::dir());
    // `#@` working context: session-scoped for v1. Pins are deliberate
    // and cheap to re-make; persisting them raises the per-cwd vs global
    // scope question, which is still open (wiki: working-context.md).
    let mut work = crate::context::WorkContext::new(cfg.engine.context_files_max_chars).with_walk(
        cfg.engine.context_tree_max_files,
        cfg.engine.context_tree_max_depth,
    );
    let mut band: Option<Band> = None;
    let mut menu: Option<Menu> = None;
    let mut chat: Option<Chat> = None;
    let mut warming: Option<String> = None;
    // Live copies of engine options the settings menu can turn.
    let mut opt_slow = cfg.engine.slow.clone();
    let mut opt_thinking = cfg.engine.thinking.clone();
    let mut opt_max_tokens = cfg.engine.max_tokens;
    let mut opt_command_first = cfg.engine.command_first;
    let mut opt_platform = cfg.engine.divulge.platform;
    let mut opt_stats = cfg.status.stats;
    let mut stats = crate::stats::Stats::new();
    // Findings that arrived while the user was browsing. The lineage
    // never mutates under an active selection, so they wait here and
    // land on the return to neutral.
    let mut held_findings: Vec<(u64, Finding)> = Vec::new();
    // The turn currently being researched, for the chrome.
    let mut researching: Option<u64> = None;
    // The fast lane has a generation in flight. Distinct from the crash
    // fuse's notion of busy, which counts every generation including
    // research, digests and warms — this one is only the `#` ask, so the
    // indicator can show the two lanes working independently.
    let mut fast_busy = false;
    // Start of the session, for time-derived animation. A frame counter
    // tied to the poll loop would mean whatever the poll timeout happened
    // to be, which is how the old insurance repaint came to fire once a
    // second by accident.
    let anim_start = Instant::now();
    // What the bound model can actually do (models.rs). None until the
    // engine reports; the UI hedges rather than lies in that window.
    let mut model_caps: Option<crate::models::Caps> = None;
    #[allow(unused_assignments)] // initialized by the first redraw! below
    let mut last_rows: Vec<String> = Vec::new();
    // Where the band sat last paint: (top row, height). Drives the
    // vacated-row erase when a resize moves it.
    let mut last_band: (u16, u16) = (0, 0);
    // Drag-resize debounce: a mac window drag fires SIGWINCH per step,
    // and repainting into a terminal that is still reflowing is how the
    // screen fills with half-drawn bands. Settle first, then draw once.
    let mut winch_at: Option<std::time::Instant> = None;
    const WINCH_SETTLE_MS: u64 = 60;
    // A paint the wrap guard skipped: the idle tick retries it. Declared
    // ahead of the macro so the macro body can reach it (hygiene binds
    // identifiers at the definition site).
    let mut paint_deferred = false;
    // Terminal-hackery knobs, live-tunable from #/debug. Copied out of
    // cfg because the menu turns them mid-session.
    let mut dbg = cfg.debug.clone();
    macro_rules! redraw {
        () => {{
            // Painting is SUSPENDED while a resize is in flight: the
            // emulator is reflowing underneath us, so anything we draw
            // lands where the band *was* a moment ago. The settle path
            // clears winch_at before repainting, so exactly one paint
            // happens per resize — after the geometry holds still.
            if winch_at.is_some() {
                // fall through; the settle repaint covers it
            } else if dbg.wrap_guard && at_last_column(parser.screen(), &layout) {
                // Deferred-wrap guard: the inner cursor is sitting in
                // the last column, the one position where interrupting
                // the line editor is provably lossy. Skip; the idle
                // repaint or the next event picks it up. Belt-and-braces
                // over cursor_save = decsc, and off by default.
                paint_deferred = true;
            } else {
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
                    chrome_tag(&work, researching).as_deref(),
                    stats_line(
                        opt_stats,
                        &mut stats,
                        &sug_hist,
                        held_findings.len(),
                        &work,
                        &ctx_log,
                        cfg.engine.num_ctx,
                    )
                    .as_deref(),
                    // Four frames a second: fast enough to read as alive,
                    // slow enough that the repaint is not chasing it.
                    &status::lane_dots(
                        fast_busy,
                        researching.is_some(),
                        ((anim_start.elapsed().as_millis() / 250) % 4) as u8,
                    ),
                );
                let pre = sync_reserved(&mut layout, &mut parser, master, rows.len() as u16);
                if !pre.is_empty() {
                    write_all(STDOUT, &pre)?;
                }
                // Hand back wherever the band USED to be. sync_reserved only
                // covers rows released when the band's height changes; a
                // terminal resize moves the whole band, and the rows it
                // vacated would otherwise keep showing a stale copy that the
                // shell has no reason to overwrite.
                let top = layout.status_row();
                let vacated =
                    reclaim_rows(last_band, top, rows.len() as u16, &layout, parser.screen());
                if !vacated.is_empty() {
                    write_all(STDOUT, &vacated)?;
                }
                last_band = (top, rows.len() as u16);
                write_all(
                    STDOUT,
                    &fixup_bytes(&layout, parser.screen(), &rows, &dbg.cursor_save),
                )?;
                last_rows = rows;
                paint_deferred = false;
            }
        }};
    }
    redraw!();
    if !typed_ahead.is_empty() {
        let _ = write_all(master, &typed_ahead);
    }

    let mut buf = [0u8; 65536];
    let mut stdin_open = true;
    let mut dirty = false;
    // Insurance-repaint state. `screen_touched` is the precondition the
    // whole mechanism insures against: output we may have mis-parsed.
    // No output since the last paint means the band cannot have been
    // damaged, so an idle session writes nothing at all.
    let mut screen_touched = false;
    let mut last_insure = Instant::now();
    let mut insure_every = INSURE_MIN;
    // The last stats line we rendered, so the row can update on its own
    // clock without a timer ever being the reason we write.
    let mut last_stats: Option<String> = None;

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
        // 30ms while dirty for quiescence-debounced redraws, and 20ms
        // while a resize settles so the drag debounce can expire.
        let timeout = PollTimeout::try_from(if winch_at.is_some() {
            20
        } else if dirty {
            30
        } else {
            250
        })
        .unwrap();
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
                    // Something reached the emulator that we may have
                    // read wrongly. That, and only that, is what arms the
                    // insurance repaint below.
                    screen_touched = true;
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
                                    // Mid-resize, layout.inner() is still the
                                    // OLD geometry — re-pinning a stale scroll
                                    // region into a terminal that just changed
                                    // size garbles it. The settle repaint pins
                                    // the correct region.
                                    if winch_at.is_none() {
                                        let mut fix =
                                            format!("\x1b[1;{}r", layout.inner().rows).into_bytes();
                                        fix.extend_from_slice(
                                            &parser.screen().cursor_state_formatted(),
                                        );
                                        write_all(STDOUT, &fix)?;
                                    }
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
                                    // A cheap stat per pin. Goulash does
                                    // not watch the filesystem and pounce
                                    // — this only sets the `*` marker, and
                                    // re-cooking stays the user's call.
                                    work.refresh_dirty();
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
                                                    question: band
                                                        .as_ref()
                                                        .and_then(|b| b.question.clone())
                                                        .unwrap_or_default(),
                                                    alt: None,
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
                                                work.context_block(),
                                                // Commentary has no question, so
                                                // the command just run stands in
                                                // as the thing to be relevant to.
                                                near_question(&memory, &work, &block.cmd),
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
                                                work.context_block(),
                                                near_question(&memory, &work, body),
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
                                    } else if let Some(rest) = body.strip_prefix('?') {
                                        // `#?` — the deliberate door to
                                        // the slow lane. Fast still
                                        // answers and still speaks; slow
                                        // researches the same turn.
                                        let rest = rest.trim();
                                        if let Some(sub) = rest.strip_prefix("/cancel") {
                                            let _ = sub;
                                            if let Some(eng) = engine.as_ref() {
                                                eng.cancel_research();
                                            }
                                            notice = Some("research cancelled".to_string());
                                        } else if let Some(sub) = rest.strip_prefix("/model") {
                                            // `#?/model` binds the research
                                            // lane. Same sigil scoping the
                                            // cancels already use: `#/x` is
                                            // global, `#?/x` is slow's.
                                            let name = sub.trim();
                                            match engine.as_ref() {
                                                Some(eng) if name.is_empty() => {
                                                    eng.list_slow_models();
                                                    let mut m = Menu::open(
                                                        "research model",
                                                        MenuKind::SlowModel,
                                                    );
                                                    m.loaded = false;
                                                    menu = Some(m);
                                                }
                                                Some(eng) => {
                                                    eng.set_slow_model(name.to_string());
                                                }
                                                None => {
                                                    notice = Some("no engine running".to_string());
                                                }
                                            }
                                        } else {
                                            // A bare `?` most likely means
                                            // "help" — tell the model that
                                            // rather than printing a card.
                                            let q = if rest.is_empty() {
                                                "The user typed a bare '?' with no question. \
                                                 They may not know the syntax and may be asking \
                                                 for help: '#' asks, '##' chats, '#@' pins a \
                                                 file, '#/' is settings, '#?' asks the slow \
                                                 model. Say so, briefly."
                                            } else {
                                                rest
                                            };
                                            let turn = next_sid;
                                            let fallback = ask_slow(
                                                q,
                                                turn,
                                                engine.as_ref(),
                                                &opt_slow,
                                                &ctx_log,
                                                memory.context_block(),
                                                work.context_block(),
                                                near_question(&memory, &work, q),
                                            );
                                            ctx_log.push_str(&format!(
                                                "# {q} [asked {}]\n",
                                                engine::hms()
                                            ));
                                            // Fast still owns the band —
                                            // it is answering this turn
                                            // like any other. Research
                                            // reports in the chrome.
                                            notice = None;
                                            band = Some(Band {
                                                question: Some(match &fallback {
                                                    Some(why) => format!("? {q} \u{2014} {why}"),
                                                    None => format!("? {q}"),
                                                }),
                                                text: "\u{2026}".to_string(),
                                            });
                                        }
                                    } else if let Some(rest) = body.strip_prefix('@') {
                                        // `#@` working context. The `/`
                                        // forms are deterministic — no
                                        // model, no ambiguity — which is
                                        // what makes them testable AND
                                        // what makes them suggestible:
                                        // `CMD: #@/path ref.md` is a
                                        // normal pullable suggestion.
                                        notice = at_command(
                                            rest,
                                            &mut work,
                                            engine.as_ref(),
                                            &last_cwd,
                                            engine_model.is_some(),
                                            &mut menu,
                                        );
                                        band = None;
                                    } else if let Some(cmdline) = body.strip_prefix('/') {
                                        // #/ commands: goulash controls, not
                                        // LLM asides. One arg max — the
                                        // single most obvious swivel.
                                        notice = slash_command(
                                            cmdline,
                                            engine.as_ref(),
                                            blocks_seen,
                                            &mut commentary,
                                            &mut memory,
                                            &mut fuse,
                                            &mut menu,
                                            &mut opt_thinking,
                                            opt_max_tokens,
                                            opt_command_first,
                                            opt_stats,
                                            model_caps.as_ref(),
                                            &dbg,
                                            &opt_slow,
                                            opt_platform,
                                        );
                                    } else if let Some(eng) = engine.as_ref() {
                                        eng.ask(
                                            body.to_string(),
                                            ctx_log.clone(),
                                            memory.context_block(),
                                            work.context_block(),
                                            near_question(&memory, &work, body),
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
                                    // Walk the FLATTENED stack, so a
                                    // researched alternative is a step of
                                    // its own rather than being
                                    // unreachable.
                                    let flat = flat_slots(&sug_hist);
                                    let cmd_at = |p: usize| {
                                        flat.get(p).and_then(|&s| slot_cmd(&sug_hist, s))
                                    };
                                    let target = if flat.is_empty() {
                                        browse = None;
                                        None
                                    } else if buffer.is_empty() {
                                        Some(0)
                                    } else {
                                        let pos = browse
                                            .filter(|&p| cmd_at(p).as_deref() == Some(&buffer))
                                            .or_else(|| {
                                                (0..flat.len()).find(|&p| {
                                                    cmd_at(p).as_deref() == Some(&buffer)
                                                })
                                            });
                                        match pos {
                                            Some(p) => Some((p + 1).min(flat.len() - 1)),
                                            None => {
                                                browse = None;
                                                None
                                            }
                                        }
                                    };
                                    if let Some(i) = target {
                                        let (ti, _) = flat[i];
                                        let mut turn = sug_hist[ti].clone();
                                        if let Some(c) = cmd_at(i) {
                                            turn.cmd = c;
                                        }
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
            winch_at = Some(std::time::Instant::now());
        }

        // Apply the resize once the drag goes quiet — and only if the
        // geometry actually moved, so a no-op SIGWINCH costs nothing.
        if let Some(at) = winch_at
            && at.elapsed() >= std::time::Duration::from_millis(WINCH_SETTLE_MS)
        {
            winch_at = None;
            if let Ok(real) = term::get_size(STDOUT)
                && real != layout.real
            {
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
                    let mut kind = MenuKind::Model;
                    let mut new_memory: Option<String> = None;
                    let mut view: Option<String> = None;
                    notice = None; // a keystroke supersedes the last outcome
                    if let Some(m) = menu.as_mut() {
                        kind = m.kind;
                        for key in parse_keys(&buf[..len]) {
                            // Composing a new entry is its own little text
                            // field: it owns every key until Enter or Esc,
                            // so typing cannot leak into the filter or
                            // move the cursor.
                            if let Some(text) = m.composing.as_mut() {
                                match key {
                                    Key::Char(c) => text.push(c),
                                    Key::Backspace => {
                                        text.pop();
                                    }
                                    Key::KillLine => text.clear(),
                                    Key::Enter => {
                                        new_memory = Some(std::mem::take(text));
                                        m.composing = None;
                                    }
                                    Key::Esc | Key::CtrlC => m.composing = None,
                                    _ => {}
                                }
                                continue;
                            }
                            // The reading pane is modal within the menu:
                            // it owns the arrows, and Esc backs out to
                            // the list rather than out of the menu.
                            if let Some(v) = m.viewing.as_mut() {
                                match key {
                                    Key::Up => v.top = v.top.saturating_sub(1),
                                    Key::Down => {
                                        v.top = (v.top + 1).min(v.lines.len().saturating_sub(1))
                                    }
                                    Key::Esc | Key::CtrlC | Key::Enter => m.viewing = None,
                                    _ => {}
                                }
                                continue;
                            }
                            match key {
                                // Esc disarms first, then closes.
                                Key::Esc | Key::CtrlC => {
                                    if m.armed.take().is_none() {
                                        close = true;
                                    }
                                }
                                Key::Enter => {
                                    let sel = m.filtered().get(m.cursor).map(|s| s.to_string());
                                    match m.kind {
                                        MenuKind::Model | MenuKind::SlowModel => {
                                            committed = sel;
                                            close = true;
                                        }
                                        MenuKind::Help => {}
                                        MenuKind::Settings | MenuKind::Debug => committed = sel,
                                        MenuKind::Pins if sel.as_deref() == Some(NEW_PIN) => {
                                            m.composing = Some(String::new());
                                            m.armed = None;
                                        }
                                        MenuKind::Memory if sel.as_deref() == Some(NEW_MEMORY) => {
                                            m.composing = Some(String::new());
                                            m.armed = None;
                                        }
                                        // Enter reads. It used to unpin,
                                        // which put a destructive verb on
                                        // the key every other menu uses
                                        // for "yes, this one".
                                        MenuKind::Memory | MenuKind::Pins => {
                                            view = sel;
                                            m.armed = None;
                                        }
                                    }
                                }
                                Key::Up => {
                                    m.armed = None;
                                    m.cursor = m.cursor.saturating_sub(1);
                                }
                                Key::Down => {
                                    m.armed = None;
                                    m.cursor += 1;
                                    m.clamp();
                                }
                                // Destructive: arm, then confirm. Delete
                                // always means delete; Backspace edits
                                // the filter while there IS one, because
                                // that is the key that built it.
                                Key::Delete | Key::Backspace
                                    if matches!(m.kind, MenuKind::Memory | MenuKind::Pins)
                                        && (matches!(key, Key::Delete) || m.filter.is_empty()) =>
                                {
                                    let sel = m.filtered().get(m.cursor).map(|s| s.to_string());
                                    let is_new = matches!(
                                        sel.as_deref(),
                                        Some(NEW_PIN) | Some(NEW_MEMORY) | None
                                    );
                                    if is_new {
                                        m.armed = None;
                                    } else if m.armed == sel {
                                        committed = sel;
                                        m.armed = None;
                                    } else {
                                        m.armed = sel;
                                    }
                                }
                                Key::Delete => m.armed = None,
                                Key::Backspace => {
                                    m.armed = None;
                                    m.filter.pop();
                                    m.clamp();
                                }
                                Key::KillLine => {
                                    m.armed = None;
                                    m.filter.clear();
                                    m.cursor = 0;
                                }
                                Key::Char(c) => {
                                    m.armed = None;
                                    m.filter.push(c);
                                    m.cursor = 0;
                                }
                            }
                        }
                    }
                    // The compose field is shared; which store it feeds
                    // depends on the menu that opened it.
                    if kind == MenuKind::Pins
                        && let Some(path) = new_memory.take()
                        && !path.trim().is_empty()
                    {
                        notice = at_command(
                            &format!("/path {}", path.trim()),
                            &mut work,
                            engine.as_ref(),
                            &last_cwd,
                            engine_model.is_some(),
                            &mut None,
                        );
                        if let Some(m) = menu.as_mut() {
                            m.items = pin_items(&work);
                            m.clamp();
                        }
                    }
                    // Enter on a pin or a slot: open it for reading. The
                    // pane shows what is actually being SENT, which is
                    // otherwise invisible from inside the session.
                    if let Some(item) = view.take() {
                        let opened = match kind {
                            MenuKind::Pins => item_id(&item).and_then(|id| work.view(id)),
                            MenuKind::Memory => item_id(&item).and_then(|id| {
                                let s = memory.slots.iter().find(|s| s.id == id)?;
                                let mut lines =
                                    vec![format!("written {} by {}", s.at, s.by), String::new()];
                                lines.extend(s.text.lines().map(|l| l.to_string()));
                                Some((format!("memory [{}]", s.id), lines))
                            }),
                            _ => None,
                        };
                        if let Some((title, lines)) = opened
                            && let Some(m) = menu.as_mut()
                        {
                            m.viewing = Some(Viewer {
                                title,
                                lines,
                                top: 0,
                            });
                        }
                    }
                    if kind == MenuKind::Pins
                        && let Some(item) = committed.take()
                    {
                        let id = item_id(&item);
                        notice = match id.and_then(|id| work.drop_id(id)) {
                            Some(label) => Some(format!("@ dropped {label}")),
                            None => Some("could not drop that pin".to_string()),
                        };
                        // A smaller pin list means a bigger share each;
                        // some of the survivors may now fit unaided.
                        kick_digests(&mut work, engine.as_ref(), engine_model.is_some());
                        if let Some(m) = menu.as_mut() {
                            m.items = pin_items(&work);
                            m.clamp();
                        }
                    }
                    if let Some(text) = new_memory {
                        notice = Some(match memory.add(&text, "user") {
                            Ok(id) => {
                                rec.memory("add", id, &text);
                                format!("remembered [{id}]")
                            }
                            Err(e) => e,
                        });
                        if let Some(m) = menu.as_mut() {
                            m.items = memory_items(&memory);
                            m.clamp();
                        }
                    }
                    if kind == MenuKind::Settings
                        && let Some(item) = committed.take()
                    {
                        // Enter cycles the value in place: apply live,
                        // persist, refresh the list under the cursor.
                        let mut parts = item.splitn(2, ':');
                        let name = parts.next().unwrap_or("").trim().to_string();
                        let cur = parts.next().unwrap_or("").trim().to_string();
                        if let Some((_, vals)) = SETTINGS.iter().find(|(n, _)| *n == name) {
                            let idx = vals.iter().position(|v| *v == cur).unwrap_or(0);
                            let next = vals[(idx + 1) % vals.len()];
                            notice = Some(format!("{name}: {next}"));
                            match name.as_str() {
                                "commentary" => {
                                    commentary = next == "on";
                                    let _ = Config::persist_key(
                                        "engine",
                                        "commentary",
                                        &commentary.to_string(),
                                    );
                                }
                                "memory" => memory.set_enabled(next == "on"),
                                "slow" => {
                                    opt_slow = next.to_string();
                                    if let Some(eng) = engine.as_ref() {
                                        eng.set_option("slow", next);
                                    }
                                    let _ = Config::persist_key("engine", "slow", next);
                                }
                                "thinking" => {
                                    opt_thinking = next.to_string();
                                    if let Some(eng) = engine.as_ref() {
                                        eng.set_option("thinking", next);
                                    }
                                    let _ = Config::persist_key("engine", "thinking", next);
                                }
                                "max_tokens" => {
                                    opt_max_tokens = next.parse().unwrap_or(opt_max_tokens);
                                    if let Some(eng) = engine.as_ref() {
                                        eng.set_option("max_tokens", next);
                                    }
                                    let _ = Config::persist_key("engine", "max_tokens", next);
                                }
                                "stats" => {
                                    opt_stats = next == "on";
                                    let _ = Config::persist_key(
                                        "status",
                                        "stats",
                                        &opt_stats.to_string(),
                                    );
                                }
                                "platform" => {
                                    opt_platform = next == "on";
                                    if let Some(eng) = engine.as_ref() {
                                        eng.set_option("platform", next);
                                    }
                                    let _ = Config::persist_key(
                                        "engine.divulge",
                                        "platform",
                                        &(next == "on").to_string(),
                                    );
                                }
                                "command_first" => {
                                    // The session vends mid-stream off its
                                    // OWN copy, so telling only the engine
                                    // left half the setting untoggled.
                                    opt_command_first = next == "on";
                                    if let Some(eng) = engine.as_ref() {
                                        eng.set_option("command_first", next);
                                    }
                                    let _ = Config::persist_key(
                                        "engine",
                                        "command_first",
                                        &(next == "on").to_string(),
                                    );
                                }
                                _ => {}
                            }
                            if let Some(m) = menu.as_mut() {
                                m.items = settings_items(&Live {
                platform: opt_platform,
                                    commentary,
                                    slow: &opt_slow,
                                    thinking: &opt_thinking,
                                    max_tokens: opt_max_tokens,
                                    command_first: opt_command_first,
                                    stats: opt_stats,
                                    memory: &memory,
                                    caps: model_caps.as_ref(),
                                });
                            }
                        }
                    }
                    if kind == MenuKind::Debug
                        && let Some(item) = committed.take()
                    {
                        // Same cycle-in-place as Settings. These apply to
                        // the very next paint — the point is to be able
                        // to watch a terminal artifact appear and vanish
                        // while the shell keeps running.
                        let mut parts = item.splitn(2, ':');
                        let name = parts.next().unwrap_or("").trim().to_string();
                        let cur = parts.next().unwrap_or("").trim().to_string();
                        if let Some((_, vals)) = DEBUG_SETTINGS.iter().find(|(n, _)| *n == name) {
                            let idx = vals.iter().position(|v| *v == cur).unwrap_or(0);
                            let next = vals[(idx + 1) % vals.len()];
                            notice = Some(format!("{name}: {next}"));
                            match name.as_str() {
                                "cursor_save" => dbg.cursor_save = next.to_string(),
                                "idle_repaint" => dbg.idle_repaint = next == "on",
                                "wrap_guard" => dbg.wrap_guard = next == "on",
                                _ => {}
                            }
                            let _ = Config::persist_key(
                                "debug",
                                &name,
                                match name.as_str() {
                                    "cursor_save" => next,
                                    _ => {
                                        if next == "on" {
                                            "true"
                                        } else {
                                            "false"
                                        }
                                    }
                                },
                            );
                            if let Some(m) = menu.as_mut() {
                                m.items = debug_items(&dbg);
                            }
                        }
                    }
                    if kind == MenuKind::Memory
                        && let Some(item) = committed.take()
                    {
                        let id = item_id(&item);
                        notice = match id {
                            Some(id) if memory.delete(id) => {
                                rec.memory("forget", id, "");
                                Some(format!("forgot [{id}]"))
                            }
                            _ => Some("could not forget that slot".to_string()),
                        };
                        if let Some(m) = menu.as_mut() {
                            m.items = memory_items(&memory);
                            m.clamp();
                        }
                    }
                    if kind == MenuKind::SlowModel
                        && let Some(name) = committed.take()
                        && let Some(eng) = engine.as_ref()
                    {
                        // Not persisted: the research lane lives in
                        // `[engine.slow_lane]`, which is a table rather
                        // than the single scalar `persist_model` knows
                        // how to rewrite. Binding it for the session is
                        // the useful half; making it stick is a config
                        // edit, and saying so beats pretending.
                        eng.set_slow_model(name.clone());
                        notice = Some(format!(
                            "research lane \u{2192} {name} (this session;                              set [engine.slow_lane] to keep it)"
                        ));
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
                        // `@` works in chat too: "pin that file" is
                        // exactly the kind of thing you say mid-
                        // conversation, and having to leave chat to do
                        // it would be the wrong seam.
                        if let Some(rest) = text.strip_prefix('?') {
                            // Same selector as at the prompt, minus the
                            // `#` we are already inside.
                            let turn = next_sid;
                            let out = ask_slow(
                                rest.trim(),
                                turn,
                                engine.as_ref(),
                                &opt_slow,
                                &ctx_log,
                                memory.context_block(),
                                work.context_block(),
                                near_question(&memory, &work, rest.trim()),
                            );
                            if let Some(c) = chat.as_mut() {
                                c.lines.push(format!("? {}", rest.trim()));
                                if let Some(msg) = out {
                                    c.lines.push(format!("goulash: {msg}"));
                                }
                            }
                        } else if let Some(rest) = text.strip_prefix('@') {
                            let out = at_command(
                                rest,
                                &mut work,
                                engine.as_ref(),
                                &last_cwd,
                                engine_model.is_some(),
                                &mut menu,
                            );
                            if let (Some(c), Some(msg)) = (chat.as_mut(), out) {
                                c.lines.push(format!("goulash: {msg}"));
                            }
                        } else if let Some(cmdline) = text.strip_prefix('/') {
                            let out = slash_command(
                                cmdline,
                                engine.as_ref(),
                                blocks_seen,
                                &mut commentary,
                                &mut memory,
                                &mut fuse,
                                &mut menu,
                                &mut opt_thinking,
                                opt_max_tokens,
                                opt_command_first,
                                opt_stats,
                                model_caps.as_ref(),
                                &dbg,
                                &opt_slow,
                                opt_platform,
                            );
                            if let (Some(c), Some(msg)) = (chat.as_mut(), out) {
                                c.lines.push(format!("goulash: {msg}"));
                            }
                        } else if let Some(eng) = engine.as_ref() {
                            if let Some(c) = chat.as_mut() {
                                c.lines.push(format!("# {text}"));
                            }
                            eng.ask(
                                text.clone(),
                                ctx_log.clone(),
                                memory.context_block(),
                                work.context_block(),
                                near_question(&memory, &work, &text),
                            );
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
                    // Measurement only — no repaint of its own. The
                    // stats row picks it up on the next paint, and a
                    // number changing is not a reason to write.
                    engine::Event::Prompt { chars } => stats.prompt_chars = chars,
                    engine::Event::Ready { provider, model } => {
                        rec.engine_ready(&provider, &model);
                        notice = Some(format!("engine: {provider} \u{b7} {model}"));
                        engine_model = Some(model);
                    }
                    // A new model is bound: its dialect and its reasoning
                    // appetite may be nothing like the last one's, so the
                    // settings menu refreshes if it is open.
                    engine::Event::Caps(caps) => {
                        model_caps = Some(caps);
                        if let Some(m) = menu.as_mut()
                            && m.kind == MenuKind::Settings
                        {
                            m.items = settings_items(&Live {
                platform: opt_platform,
                                commentary,
                                slow: &opt_slow,
                                thinking: &opt_thinking,
                                max_tokens: opt_max_tokens,
                                command_first: opt_command_first,
                                stats: opt_stats,
                                memory: &memory,
                                caps: model_caps.as_ref(),
                            });
                        }
                    }
                    // A compression landed (or failed, in which case the
                    // pin quietly keeps its outline — which is exactly
                    // why the outline had to exist first).
                    // A finding lands at its ORIGIN, never at the top of
                    // the stack. If the user has moved on, they have
                    // moved on — this is simply there when they browse
                    // back, and nothing arriving late can seize
                    // attention.
                    engine::Event::Finding {
                        turn,
                        text,
                        command,
                        reasoning,
                    } => {
                        let finding = Finding {
                            cmd: command,
                            text,
                            reasoning,
                        };
                        // The lineage never mutates under an active
                        // selection: while the user is browsing, an
                        // amendment would change the entry they are
                        // reading. Hold it and land it on return to
                        // neutral.
                        if browse.is_some() || chat.as_ref().is_some_and(|c| c.sel.is_some()) {
                            held_findings.push((turn, finding));
                        } else {
                            apply_finding(&mut sug_hist, turn, finding, &mut ctx_log);
                        }
                    }
                    engine::Event::Researching(t) => {
                        researching = t;
                    }
                    engine::Event::Digest { id, text, card } => {
                        let applied = if card {
                            work.set_card(id, text)
                        } else {
                            work.set_digest(id, text)
                        };
                        // A card landing is not worth a notice: it is a
                        // quiet improvement to a pin that was already
                        // working. A digest changes what the model sees
                        // enough to say so.
                        if let Some(msg) = applied
                            && !card
                        {
                            notice = Some(msg);
                        }
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
                    // Command-first: the CMD line lands before the prose
                    // finishes, so the suggestion is pullable while the
                    // explanation is still streaming in.
                    engine::Event::Command(cmd) => {
                        let id = next_sid;
                        next_sid += 1;
                        rec.suggest(id, &cmd, "from # ask", "engine");
                        hist_push(
                            &mut sug_hist,
                            SugTurn {
                                id,
                                cmd: cmd.clone(),
                                text: band
                                    .as_ref()
                                    .and_then(|b| b.question.clone())
                                    .unwrap_or_default(),
                                question: band
                                    .as_ref()
                                    .and_then(|b| b.question.clone())
                                    .unwrap_or_default(),
                                alt: None,
                            },
                        );
                        suggestions.insert(0, (id, cmd.clone(), "from # ask".to_string()));
                        suggestions.truncate(8);
                        ctx_log.push_str(&format!("CMD: {cmd}\n"));
                    }
                    engine::Event::Answer {
                        text,
                        command,
                        proactive,
                        remembers,
                        forgets,
                        pins,
                        pinclear,
                    } => {
                        // The generation completed: the bound model earned
                        // its trust (ends probation, clears any distrust).
                        if let Some(m) = engine_model.as_ref() {
                            fuse.promote(m);
                        }
                        // Working-context verbs. Clear first, for the same
                        // reason forgets precede remembers below: a
                        // "swap to that file" answer is PINCLEAR + PIN.
                        if pinclear && work.clear() > 0 {
                            notice = Some("@ cleared".to_string());
                        }
                        for path in &pins {
                            // Same rule as the typed form: the model's
                            // relative path means the SHELL's cwd.
                            let base = std::path::Path::new(if last_cwd.is_empty() {
                                "."
                            } else {
                                last_cwd.as_str()
                            });
                            notice = Some(match work.pin(&base.join(shellexpand(path))) {
                                Ok(msg) => msg,
                                Err(e) => format!("@ {e}"),
                            });
                        }
                        if !pins.is_empty() || pinclear {
                            kick_digests(&mut work, engine.as_ref(), engine_model.is_some());
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
                            // History mirrors the directive's shape: the
                            // model learns the format by mimicry, so a
                            // command-first contract needs command-first
                            // history. (Mid-stream vends already logged
                            // their CMD line before this point.)
                            if let Some(cmd) = &command
                                && opt_command_first
                            {
                                ctx_log.push_str(&format!("CMD: {cmd}\n"));
                            }
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
                                        question: band
                                            .as_ref()
                                            .and_then(|b| b.question.clone())
                                            .unwrap_or_default(),
                                        alt: None,
                                    },
                                );
                                suggestions.insert(0, (id, cmd.clone(), why.to_string()));
                                suggestions.truncate(8);
                                if !opt_command_first {
                                    ctx_log.push_str(&format!("CMD: {cmd}\n"));
                                }
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
                    // Not an error and not a Ready: binding the research
                    // lane is worth confirming, but the session tracks
                    // the FAST model from Ready and would start naming
                    // the wrong one.
                    engine::Event::Meter {
                        asks,
                        research,
                        digests,
                        queued,
                        backfill,
                    } => {
                        stats.asks = asks;
                        stats.research = research;
                        stats.digests = digests;
                        stats.queued = queued;
                        stats.backfill = backfill;
                    }
                    engine::Event::Notice(msg) => notice = Some(msg),
                    // Follows a #/status: the worker is the only place
                    // that knows what actually bound where.
                    engine::Event::Lanes(msg) => {
                        notice = Some(match notice.take() {
                            Some(n) => format!("{n} \u{b7} {msg}"),
                            None => msg,
                        });
                    }
                    engine::Event::Debug(raw) => rec.engine_debug(&raw),
                    engine::Event::Busy { model, warm, kind } => {
                        fuse.busy(&model);
                        if kind == engine::Work::Ask {
                            fast_busy = true;
                        }
                        if warm {
                            // A model load can take a long minute on a
                            // big model — never leave the user pinned
                            // and guessing.
                            notice = Some(format!("loading {model} \u{2026}"));
                            warming = Some(model);
                        }
                    }
                    engine::Event::Idle { kind } => {
                        fuse.idle();
                        if kind == engine::Work::Ask {
                            fast_busy = false;
                        }
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
        // sequences we don't know about, a repaint runs behind that —
        // armed by output rather than by a clock, because output is the
        // only thing that can damage the band.
        if n == 0 {
            // Stats keep their own clock. The row exists to catch runaway
            // growth, so it must not freeze exactly when the session goes
            // quiet — but it earns a write by CHANGING, never by a timer
            // expiring. `stats_line` samples at most once every five
            // seconds and rounds to whole megabytes, so a resting process
            // renders the same string and nothing is written.
            if opt_stats {
                let line = stats_line(
                    true,
                    &mut stats,
                    &sug_hist,
                    held_findings.len(),
                    &work,
                    &ctx_log,
                    cfg.engine.num_ctx,
                );
                if line != last_stats {
                    last_stats = line;
                    dirty = true;
                }
            }
            // Focus released: land anything that arrived while the user
            // was reading. Checked here rather than hooked into every
            // place browsing can end — there are six of those, and a
            // missed one would strand a finding forever.
            if !held_findings.is_empty()
                && browse.is_none()
                && chat.as_ref().is_none_or(|c| c.sel.is_none())
            {
                for (turn, finding) in held_findings.drain(..) {
                    apply_finding(&mut sug_hist, turn, finding, &mut ctx_log);
                }
                dirty = true;
            }
            if dirty || paint_deferred {
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
                    chrome_tag(&work, researching).as_deref(),
                    stats_line(
                        opt_stats,
                        &mut stats,
                        &sug_hist,
                        held_findings.len(),
                        &work,
                        &ctx_log,
                        cfg.engine.num_ctx,
                    )
                    .as_deref(),
                    &status::lane_dots(
                        fast_busy,
                        researching.is_some(),
                        ((anim_start.elapsed().as_millis() / 250) % 4) as u8,
                    ),
                );
                if rows != last_rows {
                    // Same paint as redraw!, which also erases wherever
                    // the band used to sit (band open/close moves it).
                    redraw!();
                    // A real paint is the strongest insurance there is,
                    // and it re-arms the fast cadence: whatever just
                    // changed is the likeliest thing to have disturbed
                    // the band. Disarming here is conditional on having
                    // actually WRITTEN — a compose that decided nothing
                    // changed has repaired nothing, and the band may
                    // still be damaged.
                    screen_touched = false;
                    last_insure = Instant::now();
                    insure_every = INSURE_MIN;
                }
                dirty = false;
            } else if screen_touched && last_insure.elapsed() >= insure_every {
                // Insurance repaint: it rescues a band we lost to output
                // we mis-parsed, but it writes into a stream the line
                // editor believes it owns, at a moment nothing asked for.
                // So it is armed by output — no output since the last
                // paint means the band cannot be damaged, and an idle
                // session writes nothing — and it decays, because a
                // stream that has not broken the band in thirty seconds
                // is not about to. `#/debug` turns it off entirely.
                //
                // The interval is a Duration against an Instant, not a
                // count of loop iterations: the old `idle_ticks >= 4`
                // meant "one second" only because the idle poll timeout
                // happened to be 250ms, three hundred lines away. It was
                // also a u8 that overflowed and panicked when the debug
                // toggle removed its only reset. Time is not a tick
                // count, and a counter's lifetime must not depend on a
                // debug flag.
                screen_touched = false;
                last_insure = Instant::now();
                insure_every = (insure_every * 2).min(INSURE_MAX);
                if dbg.idle_repaint && winch_at.is_none() {
                    write_all(
                        STDOUT,
                        &fixup_bytes(&layout, parser.screen(), &last_rows, &dbg.cursor_save),
                    )?;
                }
            }
        }
    }

    // Restore the terminal: full scroll region, cursor to the old status
    // row start, default attributes, visible cursor.
    // Erase EVERY reserved row, not just the first. `ESC[2K` clears one
    // line, so three-quarters of the band used to survive the exit and
    // sit under the parent shell's next prompt as debris.
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(b"\x1b[r\x1b[0m");
    for r in layout.status_row()..=layout.real.rows {
        out.extend_from_slice(format!("\x1b[{r};1H\x1b[2K").as_bytes());
    }
    out.extend_from_slice(format!("\x1b[{};1H", layout.status_row()).as_bytes());
    out.extend_from_slice(b"\x1b[?25h");
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
                Key::Delete => 'X',
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

    /// Forward delete is the one parameterized CSI we DO want, and it
    /// must not drag its neighbours in: `ESC[2~` (insert) is not it.
    #[test]
    fn forward_delete_parses_and_its_neighbours_do_not() {
        assert_eq!(kinds(b"\x1b[3~"), "X");
        assert_eq!(kinds(b"\x1b[3~\x1b[3~"), "XX");
        assert_eq!(kinds(b"\x1b[2~\x1b[5~\x1b[6~"), "");
        // A Mac's Delete key sends the backspace byte, not this.
        assert_eq!(kinds(b"\x7f"), "<");
    }

    #[test]
    fn lone_esc_and_controls() {
        assert_eq!(kinds(b"\x1b"), "E");
        assert_eq!(kinds(b"\x03\r\x7f\x15q"), "C\u{23ce}<Kq");
    }
}
#[cfg(test)]
mod view_tests {
    use super::{item_id, wrap_hard};

    /// The pane counts rows against a height; a wrap that can return
    /// nothing would spin there forever.
    #[test]
    fn every_line_yields_at_least_one_row() {
        assert_eq!(wrap_hard("", 20), vec![""]);
        assert_eq!(wrap_hard("   ", 20), vec!["   "]);
        assert!(!wrap_hard(&"x".repeat(500), 20).is_empty());
    }

    #[test]
    fn long_lines_split_and_keep_every_character() {
        let parts = wrap_hard(&"abcdefghij".repeat(5), 10);
        assert_eq!(parts.len(), 5);
        assert_eq!(parts.concat(), "abcdefghij".repeat(5));
    }

    /// Indentation is meaning in a pinned file, so it survives — unlike
    /// `wrap_chars`, which collapses whitespace for prose.
    #[test]
    fn interior_whitespace_survives_but_control_bytes_do_not() {
        assert_eq!(wrap_hard("    indented", 40), vec!["    indented"]);
        assert_eq!(wrap_hard("a\x1b[2Jb", 40), vec!["a [2Jb"]);
        assert_eq!(wrap_hard("a\tb", 40), vec!["a    b"]);
    }

    #[test]
    fn rows_carry_their_stores_id() {
        assert_eq!(item_id("[7] /path/to/thing \u{b7} 12 chars"), Some(7));
        assert_eq!(item_id("[12] a note"), Some(12));
        assert_eq!(item_id("+ pin a file \u{2026}"), None);
    }
}

#[cfg(test)]
mod band_tests {
    use super::{
        Finding, Layout, SugTurn, apply_finding, at_last_column, fixup_bytes, hist_push,
        reclaim_rows,
    };
    use crate::term::Size;

    fn layout(rows: u16, cols: u16, reserved: u16) -> Layout {
        Layout {
            real: Size { rows, cols },
            reserved,
        }
    }

    /// Row numbers named by the emitted `ESC[<n>;1H` cursor moves.
    fn rows(bytes: &[u8]) -> Vec<u16> {
        String::from_utf8_lossy(bytes)
            .split("\x1b[")
            .filter_map(|s| s.strip_suffix(";1H"))
            .filter_map(|s| s.parse().ok())
            .collect()
    }

    /// The restore has to be DECSC/DECRC, not CUP. An absolute move is
    /// the one thing that cannot carry deferred wrap across our paint,
    /// and the shell is mid-line every time we interrupt it.
    #[test]
    fn paint_saves_and_restores_with_the_terminals_own_cursor() {
        let l = layout(24, 80, 4);
        let p = vt100::Parser::new(l.inner().rows, l.inner().cols, 0);
        let out = fixup_bytes(&l, p.screen(), &["bar".to_string()], "decsc");
        let s = String::from_utf8_lossy(&out);
        assert!(s.starts_with("\x1b7"), "DECSC must come first: {s:?}");
        // No absolute cursor move survives the restore — that would
        // re-cancel the wrap flag DECRC just handed back.
        let after = s.rsplit("\x1b8").next().unwrap();
        assert!(!after.contains(";1H"), "CUP after DECRC: {after:?}");
        // Visibility is the one thing DECSC does not carry.
        assert!(
            after.contains("\x1b[?25h"),
            "visibility not restored: {after:?}"
        );

        // The escape hatch really does revert to the old shape.
        let old = fixup_bytes(&l, p.screen(), &["bar".to_string()], "absolute");
        let s = String::from_utf8_lossy(&old);
        assert!(!s.contains("\x1b7") && !s.contains("\x1b8"), "{s:?}");
    }

    #[test]
    fn wrap_guard_only_fires_in_the_final_column() {
        let l = layout(24, 80, 4);
        let mut p = vt100::Parser::new(l.inner().rows, l.inner().cols, 0);
        assert!(!at_last_column(p.screen(), &l));
        // 79 glyphs: cursor sits in column 80 of 80, deferred wrap.
        p.process("x".repeat(79).as_bytes());
        assert!(at_last_column(p.screen(), &l));
    }

    #[test]
    fn resize_taller_reclaims_the_band_left_behind() {
        // 24-row terminal, band at 21..24; grow to 30 -> band at 27..30.
        let l = layout(30, 80, 4);
        let p = vt100::Parser::new(l.inner().rows, l.inner().cols, 0);
        assert_eq!(
            rows(&reclaim_rows((21, 4), 27, 4, &l, p.screen())),
            vec![21, 22, 23, 24]
        );
    }

    #[test]
    fn overlap_is_untouched_and_offscreen_is_skipped() {
        let l = layout(24, 80, 4);
        let p = vt100::Parser::new(l.inner().rows, l.inner().cols, 0);
        // Band slid up one: the shared rows are repainted anyway, so
        // only the row it no longer covers is handed back.
        assert_eq!(
            rows(&reclaim_rows((21, 4), 20, 4, &l, p.screen())),
            vec![24]
        );
        // Shrunk terminal: the old band is off-screen, nothing to do.
        let small = layout(20, 80, 4);
        assert!(reclaim_rows((27, 4), 17, 4, &small, p.screen()).is_empty());
    }

    #[test]
    fn unchanged_band_and_first_paint_are_no_ops() {
        let l = layout(24, 80, 4);
        let p = vt100::Parser::new(l.inner().rows, l.inner().cols, 0);
        assert!(reclaim_rows((21, 4), 21, 4, &l, p.screen()).is_empty());
        assert!(reclaim_rows((0, 0), 21, 4, &l, p.screen()).is_empty());
    }

    /// The whole point: a reclaimed row gets the SHELL's content back,
    /// not a blank. The inner world only redraws at a prompt turn, so
    /// erasing what it believes is on screen leaves a lasting hole.
    #[test]
    fn reclaimed_rows_restore_shell_content_from_the_mirror() {
        let l = layout(30, 80, 4);
        let mut p = vt100::Parser::new(24, 80, 0); // inner size before the grow
        p.process(b"\x1b[21;1Hkeep-me");
        p.screen_mut().set_size(l.inner().rows, l.inner().cols);
        let mirrored = p.screen().rows_formatted(0, 80).nth(20).unwrap();
        assert!(
            String::from_utf8_lossy(&mirrored).contains("keep-me"),
            "vt100 mirror must survive a resize for reclaim to be safe"
        );
        let out = reclaim_rows((21, 4), 27, 4, &l, p.screen());
        assert!(String::from_utf8_lossy(&out).contains("keep-me"));
    }

    fn turn(id: u64, cmd: &str) -> SugTurn {
        SugTurn {
            id,
            cmd: cmd.to_string(),
            text: String::new(),
            question: String::new(),
            alt: None,
        }
    }

    #[test]
    fn dedup_moves_the_id_so_research_still_lands() {
        // Ask the same thing twice, fast vends the same command both
        // times: the second is deduped and gets no slot. Research for
        // the second ask was dispatched against id 2, and apply_finding
        // drops a finding whose turn it cannot find — so an explicit
        // `#?` on a repeated question used to produce nothing at all.
        let mut hist = vec![];
        hist_push(&mut hist, turn(1, "ls -la"));
        hist_push(&mut hist, turn(2, "ls -la"));
        assert_eq!(hist.len(), 1, "re-vending the same fix is not a new turn");
        assert_eq!(hist[0].id, 2, "but the id must follow the live ask");

        let mut log = String::new();
        apply_finding(
            &mut hist,
            2,
            Finding {
                cmd: Some("ls -lah".into()),
                text: "human sizes".into(),
                reasoning: "-h is friendlier".into(),
            },
            &mut log,
        );
        assert!(hist[0].alt.is_some(), "the finding found its home");
    }

    #[test]
    fn a_different_command_still_gets_its_own_slot() {
        let mut hist = vec![];
        hist_push(&mut hist, turn(1, "ls"));
        hist_push(&mut hist, turn(2, "pwd"));
        assert_eq!(hist.len(), 2);
        assert_eq!(hist[0].id, 2);
    }
}
