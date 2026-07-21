use crate::term::Size;

/// SGR palette for the reserved area: the agent's content block and the
/// static chrome read as visually distinct surfaces.
pub const AGENT_SGR: &str = "\x1b[0;97;44m"; // white on blue: agent content
pub const QUERY_SGR: &str = "\x1b[0;93;44m"; // yellow on blue: user question
pub const CHROME_SGR: &str = "\x1b[0;97;100m"; // white on gray: static chrome

/// The status row: agent content (suggestion/notice) fills the left in
/// the agent block color; the static chrome — identity, shell, state,
/// and geometry — sits right-justified in its own shading so it visibly
/// does not ride along with the agent party.
pub fn render(
    real: Size,
    inner_rows: u16,
    reserved: u16,
    shell_name: &str,
    state: &str,
    extra: Option<&str>,
) -> String {
    let cols = real.cols as usize;
    let chrome = format!(
        " goulash \u{2502} {shell_name} \u{2502} {state} # {}x{inner_rows}+{reserved} ",
        real.cols
    );
    let chrome_len = chrome.chars().count();
    if chrome_len >= cols {
        let clipped: String = chrome.chars().take(cols).collect();
        return format!("{CHROME_SGR}{clipped}\x1b[0m");
    }
    let content_width = cols - chrome_len;
    let content = extra.map(|e| format!(" {e} ")).unwrap_or_default();
    let mut left: String = content.chars().take(content_width).collect();
    let used = left.chars().count();
    left.push_str(&" ".repeat(content_width - used));
    format!("{AGENT_SGR}{left}{CHROME_SGR}{chrome}\x1b[0m")
}

/// One full-width styled band row (padded/truncated to cols).
pub fn pad_row(text: &str, cols: usize, sgr: &str) -> String {
    let mut line: String = text.chars().take(cols).collect();
    let used = line.chars().count();
    line.push_str(&" ".repeat(cols.saturating_sub(used)));
    format!("{sgr}{line}\x1b[0m")
}
