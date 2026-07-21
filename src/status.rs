use crate::term::Size;

/// Render the status row content for the given real-terminal size.
/// Returns the styled bytes to print at the status row; exactly one
/// terminal row wide, no trailing newline.
pub fn render(
    real: Size,
    inner_rows: u16,
    shell_name: &str,
    state: &str,
    extra: Option<&str>,
) -> String {
    let left = match extra {
        Some(e) => format!(" goulash \u{2502} {shell_name} \u{2502} {state} \u{2502} {e} "),
        None => format!(" goulash \u{2502} {shell_name} \u{2502} {state} "),
    };
    let right = format!(" {}x{}+{} ", real.cols, inner_rows, real.rows - inner_rows);
    let cols = real.cols as usize;
    let used = left.chars().count() + right.chars().count();
    let line = if used <= cols {
        format!("{left}{}{right}", " ".repeat(cols - used))
    } else {
        left.chars()
            .chain(" ".repeat(cols).chars())
            .take(cols)
            .collect()
    };
    // Inverse video bar; SGR is restored by the caller from tracked state.
    format!("\x1b[0;7m{line}\x1b[0m")
}
