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

/// Top boundary of the goulash area: a horizontal rule. An orange chip
/// for a pullable suggestion (or plain inset text for a notice) cuts in
/// at the left edge; a dim ingress tip cuts in at the right edge and
/// yields silently when space runs out.
pub fn rule_row(text: Option<&str>, chip: Option<&str>, tip: Option<&str>, cols: usize) -> String {
    // Left chip: one-dash lead-in. `chip` is the SGR it wears — orange
    // when it is the selected, pullable thing, grey when something else
    // is, and None for a plain notice that is not pullable at all.
    let (left, left_len) = match text {
        Some(t) => {
            let clipped: String = t.chars().take(cols.saturating_sub(6)).collect();
            let n = clipped.chars().count();
            let sgr = chip.unwrap_or("\x1b[0m");
            (format!("{RULE_SGR}\u{2500}{sgr}{clipped}"), n + 1)
        }
        None => (format!("{RULE_SGR}\u{2500}"), 1),
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

/// One full-width styled band row (padded/truncated to cols).
pub fn pad_row(text: &str, cols: usize, sgr: &str) -> String {
    let mut line: String = text.chars().take(cols).collect();
    let used = line.chars().count();
    line.push_str(&" ".repeat(cols.saturating_sub(used)));
    format!("{sgr}{line}\x1b[0m")
}

#[cfg(test)]
mod tests {
    use super::{Size, chrome_row, lane_dots, rule_row};

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
        let row = rule_row(None, None, Some(" tip "), 40);
        assert!(row.contains(" tip "));
        assert_eq!(width(&row), 40);
        // one-dash trail after the tip
        assert!(row.ends_with("\u{2500}\x1b[0m"));
    }

    #[test]
    fn tip_yields_when_tight() {
        let row = rule_row(Some(" a long left chip "), None, Some(" a long tip "), 24);
        assert!(!row.contains("tip"));
        assert_eq!(width(&row), 24);
    }

    #[test]
    fn left_chip_and_tip_coexist() {
        let row = rule_row(Some(" notice "), None, Some(" tip "), 40);
        assert!(row.contains(" notice ") && row.contains(" tip "));
        assert_eq!(width(&row), 40);
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
}
