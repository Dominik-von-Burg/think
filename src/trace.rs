//! The scroll trace, for a report about the writing position.

/// A per-frame trace of the notebook's scroll, for the reports where the view
/// does not land where the code says it should. It lands in the data directory
/// as `scroll.log` so it can be collected from someone who is not opening a
/// terminal; THINK_SCROLL_LOG overrides the path. One line per pass that moved
/// the caret, and one for the pass egui paints instead when that one is redone.
pub(crate) fn scroll_log(data_dir: &std::path::Path, line: &str) {
    let path = std::env::var("THINK_SCROLL_LOG")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| data_dir.join("scroll.log"));
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{line}");
    }
}
