use scraper::{ElementRef, Html, Node};

/// Elements whose text is never user-visible content.
const HIDDEN: &[&str] = &[
    "script", "style", "noscript", "template", "iframe", "object",
];
/// Elements that separate words.
const BREAKS: &[&str] = &[
    "br", "p", "div", "li", "ul", "ol", "tr", "td", "th", "h1", "h2", "h3", "h4", "h5", "h6",
    "section", "article", "table",
];

/// Decode entities, drop tags, scripts and styles, collapse whitespace and
/// control characters, and cap at `max_chars` Unicode scalar values.
pub fn clean_text(raw: &str, max_chars: usize) -> String {
    let text = if raw.contains(['<', '&']) {
        html_text(raw)
    } else {
        raw.to_owned()
    };
    let normalized = text
        .split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.chars().count() <= max_chars {
        return normalized;
    }
    // Keep room for an ellipsis so truncation is visible.
    let mut out: String = normalized
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect();
    out.truncate(out.trim_end().len());
    if max_chars > 0 {
        out.push('…');
    }
    out
}

fn html_text(raw: &str) -> String {
    let fragment = Html::parse_fragment(raw);
    let mut out = String::with_capacity(raw.len());
    collect_text(fragment.root_element(), &mut out);
    out
}

/// Visible text in document order; block elements separate words.
fn collect_text(element: ElementRef<'_>, out: &mut String) {
    for child in element.children() {
        match child.value() {
            Node::Text(text) => out.push_str(text),
            Node::Element(e) if HIDDEN.contains(&e.name()) => {}
            Node::Element(e) => {
                let block = BREAKS.contains(&e.name());
                if block {
                    out.push(' ');
                }
                if let Some(child) = ElementRef::wrap(child) {
                    collect_text(child, out);
                }
                if block {
                    out.push(' ');
                }
            }
            _ => {}
        }
    }
}

/// Keep at most `limit` items (bounded by [`super::limits::LIST_MAX`]) and
/// report whether anything was dropped.
pub fn cap_items<T>(mut items: Vec<T>, limit: usize) -> (Vec<T>, bool) {
    let limit = limit.min(super::limits::LIST_MAX);
    let truncated = items.len() > limit;
    items.truncate(limit);
    (items, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_scripts_styles_and_tags() {
        let raw =
            "<p>Quiet &amp; bright</p><script>alert('x')</script><style>p{}</style><b>room</b>";
        assert_eq!(clean_text(raw, 160), "Quiet & bright room");
    }

    #[test]
    fn decodes_entities_and_collapses_whitespace() {
        assert_eq!(clean_text("  A&nbsp;&lt;B&gt;\n\t C ", 160), "A <B> C");
    }

    #[test]
    fn caps_by_unicode_chars_with_ellipsis() {
        let out = clean_text(&"é".repeat(600), 500);
        assert_eq!(out.chars().count(), 500);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn short_text_is_unchanged() {
        assert_eq!(clean_text("Room 101", 160), "Room 101");
    }

    #[test]
    fn cap_items_respects_hard_max() {
        let (items, truncated) = cap_items((0..100).collect::<Vec<_>>(), 500);
        assert_eq!(items.len(), 50);
        assert!(truncated);
        let (items, truncated) = cap_items(vec![1, 2], 10);
        assert_eq!(items, vec![1, 2]);
        assert!(!truncated);
    }
}
