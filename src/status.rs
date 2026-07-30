use crate::term::Size;

/// Palette: most of the goulash area is plain terminal background; the
/// chrome chip is white-on-grey; the suggestion chip is orange.
pub const CHROME_SGR: &str = "\x1b[0;97;100m"; // white on grey chip
pub const SUGGEST_SGR: &str = "\x1b[0;30;48;5;208m"; // black on orange chip
pub const RULE_SGR: &str = "\x1b[0;37m"; // white rule on default bg
pub const QUERY_SGR: &str = "\x1b[0;2m"; // dim question on default bg
pub const TEXT_SGR: &str = "\x1b[0m"; // plain answer text

/// Top boundary of the goulash area: a horizontal rule. An orange chip
/// for a pullable suggestion (or plain inset text for a notice) cuts in
/// at the left edge; a dim ingress tip cuts in at the right edge and
/// yields silently when space runs out.
pub fn rule_row(text: Option<&str>, orange: bool, tip: Option<&str>, cols: usize) -> String {
    // Left chip: one-dash lead-in.
    let (left, left_len) = match text {
        Some(t) => {
            let clipped: String = t.chars().take(cols.saturating_sub(6)).collect();
            let n = clipped.chars().count();
            let sgr = if orange { SUGGEST_SGR } else { "\x1b[0m" };
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
/// Two cells of activity, right after the name.
///
/// FAST is a brief pulse; SLOW can run for tens of seconds while the
/// model reasons, so its dots CYCLE — a static mark would read as a hang
/// exactly when the user most needs to know something is happening.
/// Filled/hollow rather than colour so it survives a mono terminal.
pub fn tier_dots(slow: Option<bool>, phase: u8) -> &'static str {
    match slow {
        None => "\u{b7}\u{b7}",
        // FAST: one filled dot, no animation — it is over too fast to see.
        Some(false) => "\u{2022}\u{b7}",
        // SLOW: rotate so the pair visibly moves.
        Some(true) => match phase % 4 {
            0 => "\u{b7}\u{b7}",
            1 => "\u{2022}\u{b7}",
            2 => "\u{2022}\u{2022}",
            _ => "\u{b7}\u{2022}",
        },
    }
}

pub fn chrome_row(
    real: Size,
    inner_rows: u16,
    reserved: u16,
    shell_name: &str,
    state: &str,
    dots: &str,
) -> String {
    // Width stops one cell short of the real edge (compose_rows explains
    // why); the geometry text still reports the true column count.
    let cols = (real.cols as usize).saturating_sub(1);
    let chrome = format!(
        " goulash {dots} \u{2502} {shell_name} \u{2502} {state} # {}x{inner_rows}+{reserved} ",
        real.cols
    );
    let clipped: String = chrome.chars().take(cols).collect();
    let pad = cols.saturating_sub(clipped.chars().count());
    format!(
        "\x1b[0m\x1b[K{}{CHROME_SGR}{clipped}\x1b[0m",
        " ".repeat(pad)
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
    use super::rule_row;

    fn width(row: &str) -> usize {
        // Strip SGR sequences; what's left is the printed cells.
        let mut n = 0;
        let mut esc = false;
        for c in row.chars() {
            match (esc, c) {
                (false, '\x1b') => esc = true,
                (false, _) => n += 1,
                (true, 'm') => esc = false,
                (true, _) => {}
            }
        }
        n
    }

    #[test]
    fn tip_rides_the_right_edge() {
        let row = rule_row(None, false, Some(" tip "), 40);
        assert!(row.contains(" tip "));
        assert_eq!(width(&row), 40);
        // one-dash trail after the tip
        assert!(row.ends_with("\u{2500}\x1b[0m"));
    }

    #[test]
    fn tip_yields_when_tight() {
        let row = rule_row(Some(" a long left chip "), false, Some(" a long tip "), 24);
        assert!(!row.contains("tip"));
        assert_eq!(width(&row), 24);
    }

    #[test]
    fn left_chip_and_tip_coexist() {
        let row = rule_row(Some(" notice "), false, Some(" tip "), 40);
        assert!(row.contains(" notice ") && row.contains(" tip "));
        assert_eq!(width(&row), 40);
    }
}

#[cfg(test)]
mod tier_tests {
    use super::tier_dots;

    #[test]
    fn idle_is_hollow_and_fast_is_static() {
        assert_eq!(tier_dots(None, 0), "\u{b7}\u{b7}");
        // FAST never animates: at ~1s it would flash once and confuse.
        assert_eq!(tier_dots(Some(false), 0), tier_dots(Some(false), 7));
    }

    /// SLOW must visibly move — a static mark during a 30s think reads as
    /// a hang, which is the exact moment the user needs reassurance.
    #[test]
    fn slow_cycles_through_four_distinct_frames() {
        let f: Vec<&str> = (0..4).map(|p| tier_dots(Some(true), p)).collect();
        assert_eq!(f.len(), 4);
        assert_eq!(f.iter().collect::<std::collections::HashSet<_>>().len(), 4);
        assert_eq!(tier_dots(Some(true), 4), f[0], "wraps");
    }

    /// Two cells always, so the chrome never reflows as the tier changes.
    #[test]
    fn width_is_constant() {
        for d in [tier_dots(None, 0), tier_dots(Some(false), 0)]
            .into_iter()
            .chain((0..4).map(|p| tier_dots(Some(true), p)))
        {
            assert_eq!(d.chars().count(), 2, "{d:?}");
        }
    }
}
