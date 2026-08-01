use crate::term::Size;

/// Palette: most of the goulash area is plain terminal background; the
/// chrome chip is white-on-grey; the suggestion chip is orange.
pub const CHROME_SGR: &str = "\x1b[0;97;100m"; // white on grey chip
pub const SUGGEST_SGR: &str = "\x1b[0;30;48;5;208m"; // black on orange chip
/// An unselected chip: present, pullable, not the one you are on.
///
/// **Orange means selected, not "suggestion".** That is the whole colour
/// rule — the chip you would pull right now is orange and everything
/// else is grey, so a glance answers "what does Enter do" rather than
/// "what kind of thing is this". Categories were the wrong axis: the
/// user already knows a finding is a finding, because it is indented
/// under the answer it fills in.
/// (wiki: architecture/two-lane-engagement.md)
pub const IDLE_CHIP_SGR: &str = "\x1b[0;97;48;5;238m";
pub const RULE_SGR: &str = "\x1b[0;37m"; // white rule on default bg
pub const QUERY_SGR: &str = "\x1b[0;2m"; // dim question on default bg
pub const TEXT_SGR: &str = "\x1b[0m"; // plain answer text

/// A text field's insertion point, inside a chip that already set its
/// own colours: blink on, the bar, blink off (`25`, not a full reset —
/// resetting here would drop the chip's background for the rest of the
/// row). A terminal without SGR 5 draws a solid bar, which is what the
/// caret was before, so there is nothing to fall back to.
pub const CARET: &str = "\x1b[5m\u{258f}\x1b[25m";

/// The working marquee wears its lane's colour on the chip background,
/// so it reads as part of the chip rather than a second widget.
pub const WORKING_FAST_SGR: &str = "\x1b[0;96;48;5;238m";
pub const WORKING_SLOW_SGR: &str = "\x1b[0;93;48;5;238m";

/// The working bar's comet tail: four intensities from the head back.
///
/// A single solid glyph jumping one whole cell per frame is all a
/// terminal can do positionally, and it reads as chunky because there is
/// nothing between the two positions for the eye to interpolate. Shading
/// the trail gives it that: as the head advances, every cell behind it
/// steps down one level, so the *pattern* moves continuously even though
/// each glyph still snaps to its cell.
const TRAIL_FAST: [&str; 7] = [
    "\x1b[0;1;97;48;5;39m",  // head: white on bright cyan
    "\x1b[0;1;97;48;5;32m",
    "\x1b[0;96;48;5;31m",
    "\x1b[0;96;48;5;24m",
    "\x1b[0;36;48;5;23m",
    "\x1b[0;36;48;5;238m",
    "\x1b[0;90;48;5;238m",   // tail, fading into the chip
];
const TRAIL_SLOW: [&str; 7] = [
    "\x1b[0;1;97;48;5;214m", // head: white on gold
    "\x1b[0;1;97;48;5;172m",
    "\x1b[0;93;48;5;136m",
    "\x1b[0;93;48;5;94m",
    "\x1b[0;33;48;5;58m",
    "\x1b[0;33;48;5;238m",
    "\x1b[0;90;48;5;238m",
];
/// Seven steps of shade to match seven of colour. Glyphs are coarse —
/// four is all Unicode gives — so the extra resolution comes from the
/// palette, and the two are indexed together so a cell's shade and hue
/// always agree.
const TRAIL_GLYPH: [&str; 7] = [
    "\u{2588}", "\u{2588}", "\u{2593}", "\u{2593}", "\u{2592}", "\u{2591}", "\u{2591}",
];

/// One styled run inside a chip: the text, and the SGR it wears.
///
/// A chip is a list of these rather than one string and one colour
/// because the suggestion chip says two different things at once: the
/// label is an affordance (orange: Down reaches this) and the command
/// is content (grey until you have actually pulled it). Painting both
/// orange was most of the "too much orange" — the eye had nothing to
/// look at because everything was shouting.
pub type Seg<'a> = (&'a str, &'a str);

/// Top boundary of the goulash area: a horizontal rule. A chip cuts in
/// at the left edge; a dim ingress tip cuts in at the right edge and
/// yields silently when space runs out.
///
/// An empty `chip` leaves a bare rule. The SGR runs are emitted between
/// segments and never counted as width — the clip budget is spent on
/// printed characters only, which is why the segments arrive separated
/// rather than pre-styled into one string.
pub fn rule_row(chip: &[Seg], tip: Option<&str>, cols: usize) -> String {
    let (left, left_len) = if chip.is_empty() {
        (format!("{RULE_SGR}\u{2500}"), 1)
    } else {
        let mut s = format!("{RULE_SGR}\u{2500}");
        let mut room = cols.saturating_sub(6);
        let mut n = 0;
        for (text, sgr) in chip {
            if room == 0 {
                break;
            }
            let clipped: String = text.chars().take(room).collect();
            let used = clipped.chars().count();
            room -= used;
            n += used;
            s.push_str(sgr);
            s.push_str(&clipped);
        }
        (s, n + 1)
    };
    // Right tip: one-dash trail-out, dropped when the row is tight.
    if let Some(t) = tip {
        let tlen = t.chars().count();
        if left_len + tlen + 2 <= cols {
            let mid = cols - left_len - tlen - 1;
            return format!(
                "{left}{RULE_SGR}{}{QUERY_SGR}{t}{RULE_SGR}\u{2500}\x1b[0m",
                "\u{2500}".repeat(mid)
            );
        }
    }
    format!(
        "{left}{RULE_SGR}{}\x1b[0m",
        "\u{2500}".repeat(cols.saturating_sub(left_len))
    )
}

/// Bottom row of the goulash area: terminal-default background with the
/// static chrome — identity, shell, state, `#` geometry — right-justified
/// in its own grey chip.
/// Bright cyan for the fast lane, amber for the slow one, dim for a lane
/// that is not working. Amber rather than the suggestion orange (208):
/// orange already means *selected* everywhere else in the band, and a
/// colour that means two things means neither.
const FAST_SGR: &str = "\x1b[0;96;100m";
const SLOW_SGR: &str = "\x1b[0;93;100m";
const OFF_SGR: &str = "\x1b[0;90;100m";

/// Two dots, left FAST and right SLOW, each animating only while its own
/// lane is working.
///
/// One dot per lane rather than one shared animation, because **both
/// lanes run at once**: fast answers while slow researches underneath
/// the same turn. A single indicator would have to pick one to report,
/// and the interesting moment — slow still working after fast has
/// finished — is exactly the one it would hide.
///
/// `phase` comes from elapsed time, not a loop counter. The loop's
/// period is an implementation detail three hundred lines away, and
/// tying an animation to it is how the old insurance repaint ended up
/// meaning "once a second" by accident.
pub fn lane_dots(fast: bool, slow: bool, phase: u8) -> String {
    // Offset the two lanes by half a cycle so simultaneous work reads as
    // two independent things, not one wide blink.
    let f = dot(fast, phase);
    let s = dot(slow, phase.wrapping_add(2));
    format!(
        "{}{f}{}{s}\x1b[0m",
        if fast { FAST_SGR } else { OFF_SGR },
        if slow { SLOW_SGR } else { OFF_SGR },
    )
}

/// The working bar: an answer you asked for is still on its way, so the
/// command in the slot belongs to the PREVIOUS question.
///
/// That is the dangerous state and the reason this exists. The lane dots
/// say a lane is busy; they do not say the thing you are about to pull
/// is stale. Down on a held suggestion during a new generation puts a
/// command you did not ask for on your prompt line, looking exactly like
/// one you did.
///
/// Three acts, all a pure function of elapsed time:
///
/// - **grow** — slides out to full width over `GROW_MS`, pushing the
///   chip right, so the arrival is what catches the eye rather than a
///   blink appearing in place;
/// - **sweep** — a bright head ping-pongs along the body. Movement with
///   no progress claim: goulash cannot know how far along a generation
///   is, and a bar that pretended to would be lying;
/// - **shrink** — snaps back in `SHRINK_MS`, about a third of the grow,
///   because the answer landing should feel like a release.
///
/// Returns `None` once the shrink is done, which is also the caller's
/// signal to stop asking for repaints.
const CELLS: usize = 5;
/// Longer than they feel they should be, because the eye reads the
/// ENTRANCE and the exit, not the steady sweep. At 180ms/70ms the bar
/// arrived and left in one or two frames — a pop, not a slide.
/// Defaults live in `DebugConfig`; these are the shapes of the dials.
/// The exit is deliberately quicker than the entrance — an answer
/// landing should feel like a release.
const SHRINK_RATIO: f32 = 0.7;

/// The bar, as styled runs. `None` once the shrink has finished, which
/// is also the caller's signal to stop asking for frames.
///
/// Cell positions are all a terminal has, so smoothness comes from
/// SHADE rather than position: the leading edge fades in through
/// `░▒▓█` as it grows and back out as it shrinks, giving each cell four
/// intermediate states instead of popping between present and absent.
pub fn working_bar(
    ms: u64,
    running: bool,
    fast: bool,
    step_ms: u64,
    grow_ms: u64,
) -> Option<Vec<Seg<'static>>> {
    let grow_ms = grow_ms.max(1);
    let shrink_ms = ((grow_ms as f32 * SHRINK_RATIO) as u64).max(1);
    let step_ms = step_ms.max(1);
    let extent: f32 = if running {
        if ms >= grow_ms {
            CELLS as f32
        } else {
            CELLS as f32 * (ms as f32 / grow_ms as f32)
        }
    } else {
        let t = ms as f32 / shrink_ms as f32;
        if t >= 1.0 {
            return None;
        }
        CELLS as f32 * (1.0 - t)
    };
    let full = extent.floor() as usize;
    let frac = extent - full as f32;
    // While RUNNING there is always at least one cell. At ms=0 the
    // extent is genuinely zero, and returning None there would tell the
    // caller the shrink had finished — cancelling the animation on its
    // own first frame.
    let width = (full + usize::from(frac > 0.02)).max(usize::from(running));
    if width == 0 {
        return None;
    }
    let trail = if fast { &TRAIL_FAST } else { &TRAIL_SLOW };
    // A FLOAT position, ping-ponging. Integer steps were the chunk: at
    // 150ms a cell the head sat still for nine frames and then jumped,
    // so every frame in between rendered identically and the wave read
    // as a strobe. Continuous position means each frame shades slightly
    // differently — the glyphs still snap to their cells, but the
    // brightness slides, and the eye reads the slide.
    //
    // Reversing rather than wrapping: a head that teleports from the
    // right edge back to the left reads as a glitch, not a sweep.
    let head: f32 = if running && width > 1 {
        let span = (width - 1) as f32 * 2.0;
        let p = (ms as f32 / step_ms as f32) % span;
        if p < (width - 1) as f32 { p } else { span - p }
    } else {
        0.0
    };
    Some(
        (0..width)
            .map(|i| {
                // Distance from the FLOAT head, scaled across the
                // seven levels: a cell one and a half away is dimmer
                // than one exactly one away, which is the whole source
                // of the between-step motion.
                const LAST: usize = TRAIL_GLYPH.len() - 1;
                let mut d = if running {
                    let dist = (i as f32 - head).abs() * 1.6;
                    (dist.round() as usize).min(LAST)
                } else {
                    LAST - 1
                };
                // ...and the leading cell is dimmed further by how much
                // of it has actually arrived, so growth and shrink read
                // as a fade rather than a jump.
                if i == full {
                    let arrived = (frac * LAST as f32) as usize;
                    d = d.max(LAST.saturating_sub(arrived));
                }
                (TRAIL_GLYPH[d], trail[d])
            })
            .collect(),
    )
}

fn dot(active: bool, phase: u8) -> char {
    if !active {
        return '\u{b7}'; // ·
    }
    match phase % 4 {
        0 => '\u{2022}', // •
        1 => '\u{25cf}', // ●
        2 => '\u{2022}', // •
        _ => '\u{b7}',   // ·
    }
}

pub fn chrome_row(
    real: Size,
    inner_rows: u16,
    reserved: u16,
    shell_name: &str,
    state: &str,
    pin: Option<&str>,
    stats: Option<&str>,
    dots: &str,
) -> String {
    // Width stops one cell short of the real edge (compose_rows explains
    // why); the geometry text still reports the true column count.
    let cols = (real.cols as usize).saturating_sub(1);
    // The active `#@` sits next to the shell name: it changes what the
    // model knows, so it has to be visible without asking.
    let at = match pin {
        Some(p) => format!(" \u{2502} {p}"),
        None => String::new(),
    };
    // Stats lead. They are the reason the row was asked for, and the
    // geometry on the right is the part that can be spared when a narrow
    // terminal clips — you can always resize to see it again, whereas a
    // number you are watching climb is the whole point.
    let diag = match stats {
        Some(s) => format!("{s} \u{2502} "),
        None => String::new(),
    };
    let chrome = format!(
        " {diag}goulash \u{2502} {shell_name}{at} \u{2502} {state} # {}x{inner_rows}+{reserved} ",
        real.cols
    );
    // The dots lead the chip. Their width is reserved BEFORE clipping —
    // `chrome` is measured in chars, so an SGR sequence inside it would
    // be counted as text and silently eat real columns.
    const LEAD: usize = 3; // one space plus two dot cells
    let room = cols.saturating_sub(LEAD);
    let clipped: String = chrome.chars().take(room).collect();
    let pad = room.saturating_sub(clipped.chars().count());
    format!(
        "\x1b[0m\x1b[K{}{CHROME_SGR} {dots}{CHROME_SGR}{clipped}\x1b[0m",
        " ".repeat(pad)
    )
}

/// The inset row for a researched finding: the question's remaining
/// words stay on the terminal's own background, and the finding's block
/// starts after them.
///
/// Full-width colour read as a takeover; indenting it keeps the row
/// legible as *an addition under a question* rather than a banner. The
/// stub is generous enough (20 chars) to still say which question.
pub fn inset_row(stub: &str, body: &str, cols: usize, selected: bool) -> String {
    let stub: String = stub.chars().take(cols / 3).collect();
    let room = cols.saturating_sub(stub.chars().count());
    let mut text: String = body.chars().take(room).collect();
    text.push_str(&" ".repeat(room.saturating_sub(text.chars().count())));
    let sgr = if selected { SUGGEST_SGR } else { IDLE_CHIP_SGR };
    format!("{QUERY_SGR}{stub}\x1b[0m{sgr}{text}\x1b[0m")
}

/// A band row with an explanatory note pushed to the right edge.
///
/// The note is dimmed and yields entirely when the row is tight: it is
/// the least important thing on the line, and a note that pushed the
/// setting's own value off the screen would be worse than no note.
pub fn pad_row_with_note(text: &str, note: &str, cols: usize, sgr: &str) -> String {
    let used = text.chars().count();
    // 2 spaces of gap, and only bother if a useful amount of the note
    // survives — a three-character stub is noise, not help.
    if note.is_empty() || used + note.chars().count() + 3 > cols {
        return pad_row(text, cols, sgr);
    }
    let gap = cols - used - note.chars().count() - 1;
    format!(
        "{sgr}{text}{}\x1b[2m{note}\x1b[22m {}\x1b[0m",
        " ".repeat(gap),
        ""
    )
}

/// One full-width styled band row (padded/truncated to cols).
pub fn pad_row(text: &str, cols: usize, sgr: &str) -> String {
    let mut line: String = text.chars().take(cols).collect();
    let used = line.chars().count();
    line.push_str(&" ".repeat(cols.saturating_sub(used)));
    format!("{sgr}{line}\x1b[0m")
}

#[cfg(test)]
mod tests {
    use super::{
        IDLE_CHIP_SGR, SUGGEST_SGR, Size, TEXT_SGR, chrome_row, lane_dots, pad_row_with_note,
        rule_row, working_bar,
    };

    fn width(row: &str) -> usize {
        // Strip escape sequences; what's left is the printed cells.
        //
        // A CSI run ends at its FINAL byte, anywhere in 0x40..=0x7E — not
        // at `m`. Ending it only on `m` worked while the only sequences
        // here were SGR, then silently swallowed the whole row the first
        // time one carried `\x1b[K`, reporting a width of nearly zero.
        let mut n = 0;
        let mut esc = false;
        for c in row.chars() {
            match (esc, c) {
                (false, '\x1b') => esc = true,
                (false, _) => n += 1,
                (true, '[') | (true, '0'..='9') | (true, ';') | (true, '?') => {}
                (true, _) => esc = false,
            }
        }
        n
    }

    #[test]
    fn tip_rides_the_right_edge() {
        let row = rule_row(&[], Some(" tip "), 40);
        assert!(row.contains(" tip "));
        assert_eq!(width(&row), 40);
        // one-dash trail after the tip
        assert!(row.ends_with("\u{2500}\x1b[0m"));
    }

    #[test]
    fn tip_yields_when_tight() {
        let row = rule_row(&[(" a long left chip ", SUGGEST_SGR)], Some(" a long tip "), 24);
        assert!(!row.contains("tip"));
        assert_eq!(width(&row), 24);
    }

    #[test]
    fn left_chip_and_tip_coexist() {
        let row = rule_row(&[(" notice ", TEXT_SGR)], Some(" tip "), 40);
        assert!(row.contains(" notice ") && row.contains(" tip "));
        assert_eq!(width(&row), 40);
    }

    /// The suggestion chip wears two colours: the label says Down
    /// reaches this, the command says whether you have taken it. Both
    /// runs are on the row, and the SGR between them costs no columns —
    /// a chip measured as text would eat real cells and drag the rule
    /// off the right edge.
    /// Grow, sweep, snap back — and above all, END. The bar is the only
    /// thing in the band that writes without the user or the shell
    /// having done something, so a shrink that never returns None is an
    /// idle session repainting forever.
    #[test]
    fn the_working_bar_grows_sweeps_and_finishes() {
        const STEP: u64 = 60;
        const GROW: u64 = 340;
        let bar = |ms, run| working_bar(ms, run, true, STEP, GROW);
        let width = |ms, run| bar(ms, run).map_or(0, |b| b.len());

        assert_eq!(width(0, true), 1, "starts small");
        assert_eq!(width(GROW + 10, true), 5, "grows to full");
        assert_eq!(width(9_999, true), 5, "and stays");

        // The head is the brightest cell; it moves, and it reverses
        // rather than wrapping.
        let head_at = |ms: u64| {
            bar(ms, true)
                .unwrap()
                .iter()
                .position(|(g, _)| *g == "\u{2588}")
                .unwrap()
        };
        let heads: Vec<usize> = (0..14).map(|i| head_at(GROW + 20 + i * STEP)).collect();
        assert!(heads.windows(2).any(|w| w[1] > w[0]), "sweeps out: {heads:?}");
        assert!(heads.windows(2).any(|w| w[1] < w[0]), "and back: {heads:?}");
        assert!(heads.iter().all(|h| *h <= 4), "stays inside: {heads:?}");

        // Shrinking is monotone and terminates.
        let widths: Vec<usize> = (0..8).map(|i| width(i * 30, false)).collect();
        assert!(widths.windows(2).all(|w| w[1] <= w[0]), "monotone: {widths:?}");
        assert_eq!(bar(10_000, false), None, "the shrink must END");

        // The dials are honoured, not decorative.
        assert_eq!(width(100, true), 2, "grow tracks grow_ms");
        assert_eq!(
            working_bar(100, true, true, STEP, 1_000).map_or(0, |b| b.len()),
            1,
            "a slower grow is narrower at the same instant"
        );

        // Sub-cell fade: the leading cell arrives faint and firms up,
        // which is what stops the entrance reading as a pop.
        let lead = |ms: u64| bar(ms, true).unwrap().last().unwrap().0;
        assert_eq!(lead(GROW / 5 + 4), "\u{2591}", "leading cell starts faint");
        assert_ne!(lead(GROW / 5 * 2 - 6), "\u{2591}", "and firms up");
    }

    #[test]
    fn a_two_tone_chip_keeps_its_width() {
        let one = rule_row(&[(" \u{2193} suggestion: du -sh * ", SUGGEST_SGR)], None, 60);
        let two = rule_row(
            &[
                (" \u{2193} suggestion: ", SUGGEST_SGR),
                ("du -sh * ", IDLE_CHIP_SGR),
            ],
            None,
            60,
        );
        assert_eq!(width(&one), 60);
        assert_eq!(width(&two), 60, "the second SGR must not count as text");
        assert!(two.contains(IDLE_CHIP_SGR) && two.contains(SUGGEST_SGR));
    }

    /// Clipping is a budget over PRINTED characters, spent across the
    /// segments in order — so a long command loses its tail and the
    /// label it was hanging off of survives.
    #[test]
    fn a_long_command_clips_without_eating_the_label() {
        let long = "x".repeat(200);
        let row = rule_row(
            &[(" \u{2193} suggestion: ", SUGGEST_SGR), (&long, IDLE_CHIP_SGR)],
            None,
            40,
        );
        assert_eq!(width(&row), 40);
        assert!(row.contains("suggestion: "));
    }

    /// Dim when nothing is working — the row is always on screen, so an
    /// idle indicator must not read as activity.
    #[test]
    fn idle_lanes_show_two_dim_dots() {
        for phase in 0..4 {
            let d = lane_dots(false, false, phase);
            assert_eq!(width(&d), 2, "always two cells");
            assert_eq!(d.matches('\u{b7}').count(), 2, "both dim at phase {phase}");
        }
    }

    /// Each lane drives its OWN dot. The case that matters is slow still
    /// working after fast has finished: the right dot must move while the
    /// left one is dim.
    #[test]
    fn each_lane_animates_only_its_own_dot() {
        let fast_frames: Vec<char> = (0..4).map(|p| lane_dots(true, false, p).chars().filter(|c| "\u{b7}\u{2022}\u{25cf}".contains(*c)).next().unwrap()).collect();
        assert!(fast_frames.iter().collect::<std::collections::HashSet<_>>().len() > 1,
                "fast dot must change over a cycle: {fast_frames:?}");

        for p in 0..4 {
            let only_slow = lane_dots(false, true, p);
            let cells: Vec<char> = only_slow.chars().filter(|c| "\u{b7}\u{2022}\u{25cf}".contains(*c)).collect();
            assert_eq!(cells.len(), 2);
            assert_eq!(cells[0], '\u{b7}', "left dot idle while only slow works");
        }
    }

    /// Both lanes run at once. Offsetting them by half a cycle is what
    /// keeps that readable as two things rather than one wide blink.
    #[test]
    fn simultaneous_lanes_are_out_of_phase() {
        let differ = (0..4).filter(|p| {
            let c: Vec<char> = lane_dots(true, true, *p).chars()
                .filter(|c| "\u{b7}\u{2022}\u{25cf}".contains(*c)).collect();
            c[0] != c[1]
        }).count();
        assert!(differ >= 2, "dots should disagree for most of the cycle");
    }

    /// The dots are coloured, so their escape sequences must not be
    /// counted as columns — that is exactly how a status row comes to
    /// overflow into the terminal's last cell and trigger a reflow.
    #[test]
    fn chrome_row_reserves_the_dots_without_counting_their_escapes() {
        let size = Size { rows: 24, cols: 80 };
        for (f, sl) in [(false, false), (true, false), (false, true), (true, true)] {
            let row = chrome_row(size, 20, 4, "zsh", "prompt", None, None, &lane_dots(f, sl, 1));
            assert_eq!(width(&row), 79, "must stop one short of the real edge");
        }
    }

    /// A narrow terminal clips the text, never the indicator.
    #[test]
    fn dots_survive_a_narrow_terminal() {
        let size = Size { rows: 24, cols: 20 };
        let row = chrome_row(size, 16, 4, "zsh", "prompt", None, None, &lane_dots(true, true, 1));
        assert_eq!(width(&row), 19);
        assert!(row.chars().any(|c| c == '\u{2022}' || c == '\u{25cf}'));
    }

    /// The note rides the right edge and the row still measures exactly
    /// `cols` — an off-by-one here writes the terminal's last cell,
    /// which flags the row as soft-wrapped and makes a resize reflow it
    /// up the scrollback.
    #[test]
    fn a_note_keeps_the_row_exactly_full_width() {
        for cols in [40usize, 80, 157] {
            let row = pad_row_with_note(" thinking: off", "reasoning effort", cols, TEXT_SGR);
            assert_eq!(width(&row), cols, "at {cols} cols");
        }
    }

    /// When there is no room, the note goes rather than the setting.
    #[test]
    fn a_tight_row_drops_the_note_not_the_value() {
        let row = pad_row_with_note(" thinking: off", "a very long explanation indeed", 24, TEXT_SGR);
        assert_eq!(width(&row), 24);
        assert!(row.contains("thinking: off"), "the value survives");
        assert!(!row.contains("explanation"), "the note yields");
    }
}
