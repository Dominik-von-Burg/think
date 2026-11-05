//! Markdown in a plain-text document: headers sized by the layouter, links found
//! in the text.

use std::sync::Arc;

use eframe::egui::{
    self,
    text::{LayoutJob, TextFormat, TextWrapping},
    Color32, FontId,
};

pub(crate) fn markdown_layouter(ui: &egui::Ui, text: &str, wrap_width: f32) -> Arc<egui::Galley> {
    let font_id = FontId::monospace(14.0);
    let regular = ui.visuals().widgets.inactive.text_color();
    let header_blue = Color32::from_rgb(140, 180, 232);
    let link_color = Color32::from_rgb(150, 166, 188);

    let mut job = LayoutJob::default();
    job.wrap = TextWrapping {
        max_width: wrap_width,
        ..Default::default()
    };

    let push = |job: &mut LayoutJob, s: &str, header: bool| {
        if s.is_empty() {
            return;
        }
        let base = if header { header_blue } else { regular };
        let mut last = 0;
        let add = |job: &mut LayoutJob, piece: &str, link: bool| {
            if piece.is_empty() {
                return;
            }
            job.append(
                piece,
                0.0,
                TextFormat {
                    font_id: font_id.clone(),
                    color: if link { link_color } else { base },
                    underline: if link {
                        egui::Stroke::new(1.0_f32, link_color)
                    } else {
                        egui::Stroke::NONE
                    },
                    ..Default::default()
                },
            );
        };
        for (a, b) in find_links(s) {
            add(job, &s[last..a], false);
            add(job, &s[a..b], true);
            last = b;
        }
        add(job, &s[last..], false);
    };

    // Merge consecutive lines of the same style into one section so egui
    // hashes a handful of sections per frame instead of one per line.
    let (mut run_start, mut pos, mut run_header) = (0, 0, false);
    for line in text.split_inclusive('\n') {
        let header = is_markdown_header(line.trim_end_matches(['\n', '\r']));
        if header != run_header && pos > run_start {
            push(&mut job, &text[run_start..pos], run_header);
            run_start = pos;
        }
        run_header = header;
        pos += line.len();
    }
    push(&mut job, &text[run_start..], run_header);

    ui.fonts(|f| f.layout_job(job))
}

/// Byte ranges of http(s) URLs in `text`.
pub(crate) fn find_links(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(rel) = text[from..].find("http") {
        let start = from + rel;
        let rest = &text[start..];
        if !(rest.starts_with("http://") || rest.starts_with("https://")) {
            from = start + 4;
            continue;
        }
        let mut end = start
            + rest
                .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"'))
                .unwrap_or(rest.len());
        while end > start
            && matches!(
                text.as_bytes()[end - 1],
                b'.' | b',' | b';' | b':' | b'!' | b'?' | b')' | b']' | b'\''
            )
        {
            end -= 1;
        }
        if end > start + 8 {
            out.push((start, end));
        }
        from = end.max(start + 4);
    }
    out
}

/// The URL under character index `idx`, if any.
pub(crate) fn link_at_char(text: &str, idx: usize) -> Option<String> {
    let byte = text.char_indices().nth(idx).map_or(text.len(), |(b, _)| b);
    find_links(text)
        .into_iter()
        .find(|&(a, b)| byte >= a && byte < b)
        .map(|(a, b)| text[a..b].to_string())
}

pub(crate) fn is_markdown_header(line: &str) -> bool {
    let line = line.trim_start_matches([' ', '\t']);
    let hashes = line.bytes().take_while(|byte| *byte == b'#').count();
    (1..=6).contains(&hashes)
        && line
            .as_bytes()
            .get(hashes)
            .is_none_or(|byte| byte.is_ascii_whitespace())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_headers_are_detected() {
        assert!(is_markdown_header("# Title"));
        assert!(is_markdown_header("###   Spaced"));
        assert!(is_markdown_header("###### Six"));
        assert!(is_markdown_header("  ## Indented"));
        assert!(!is_markdown_header("#nospace"));
        assert!(!is_markdown_header("####### Seven"));
        assert!(!is_markdown_header("plain text"));
        assert!(!is_markdown_header(""));
    }
}
