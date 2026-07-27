use crate::term::Size;

/// Palette: most of the goulash area is plain terminal background; the
/// chrome chip is white-on-grey; the suggestion chip is orange.
pub const CHROME_SGR: &str = "\x1b[0;97;100m"; // white on grey chip
pub const SUGGEST_SGR: &str = "\x1b[0;30;48;5;208m"; // black on orange chip
/// A researched finding, inset under the answer it fills in. Deep royal
/// blue: it recedes against the suggestion orange (208) instead of
/// competing the way a brighter blue would, and white carries the
/// contrast that orange gets from black text.
/// (wiki: architecture/two-lane-engagement.md)
pub const RESEARCH_SGR: &str = "\x1b[0;97;48;5;25m";
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
pub fn chrome_row(
    real: Size,
    inner_rows: u16,
    reserved: u16,
    shell_name: &str,
    state: &str,
    pin: Option<&str>,
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
    let chrome = format!(
        " goulash \u{2502} {shell_name}{at} \u{2502} {state} # {}x{inner_rows}+{reserved} ",
        real.cols
    );
    let clipped: String = chrome.chars().take(cols).collect();
    let pad = cols.saturating_sub(clipped.chars().count());
    format!(
        "\x1b[0m\x1b[K{}{CHROME_SGR}{clipped}\x1b[0m",
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
    // Selected reads as the pullable thing (orange, like any chip);
    // unselected is the quieter blue that says "there is more here".
    let sgr = if selected { SUGGEST_SGR } else { RESEARCH_SGR };
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
