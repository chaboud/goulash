use crate::term::Size;

/// Palette: most of the goulash area is plain terminal background; the
/// chrome chip is white-on-grey; the suggestion chip is orange.
pub const CHROME_SGR: &str = "\x1b[0;97;100m"; // white on grey chip
pub const SUGGEST_SGR: &str = "\x1b[0;30;48;5;208m"; // black on orange chip
pub const RULE_SGR: &str = "\x1b[0;37m"; // white rule on default bg
pub const QUERY_SGR: &str = "\x1b[0;2m"; // dim question on default bg
pub const TEXT_SGR: &str = "\x1b[0m"; // plain answer text

/// Top boundary of the goulash area: a horizontal rule with optional text
/// cutting in near the right edge — orange chip for a pullable
/// suggestion, plain inset text for notices.
pub fn rule_row(text: Option<&str>, orange: bool, cols: usize) -> String {
    let Some(t) = text else {
        return format!("{RULE_SGR}{}\x1b[0m", "\u{2500}".repeat(cols));
    };
    let clipped: String = t.chars().take(cols.saturating_sub(6)).collect();
    let tlen = clipped.chars().count();
    let trail = cols.saturating_sub(tlen + 2);
    let mid = if orange {
        format!("{SUGGEST_SGR}{clipped}")
    } else {
        format!("\x1b[0m{clipped}")
    };
    // Chip at the left edge: short lead-in, rule fills to the right.
    format!(
        "{RULE_SGR}{}{mid}{RULE_SGR}{}\x1b[0m",
        "\u{2500}".repeat(2),
        "\u{2500}".repeat(trail)
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
) -> String {
    let cols = real.cols as usize;
    let chrome = format!(
        " goulash \u{2502} {shell_name} \u{2502} {state} # {}x{inner_rows}+{reserved} ",
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
