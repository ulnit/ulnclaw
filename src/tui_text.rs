//! Shared text-layout helpers for raw-mode terminal surfaces (session
//! browse detail pane and friends).

/// Greedy word-wrap over Unicode text (char-width based, no ANSI
/// awareness — callers strip styling before wrapping). Paragraphs split
/// on `\n`; whitespace runs collapse to single spaces; words longer than
/// `width` hard-break.
pub fn wrap_display_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out: Vec<String> = Vec::new();
    for paragraph in text.split('\n') {
        let mut pending: std::collections::VecDeque<String> = paragraph
            .split_whitespace()
            .map(|w| w.to_string())
            .collect();
        if pending.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut line = String::new();
        let mut len = 0usize;
        while let Some(word) = pending.pop_front() {
            let word_len = word.chars().count();
            if word_len > width {
                if len > 0 {
                    out.push(std::mem::take(&mut line));
                    len = 0;
                }
                let split_at = word
                    .char_indices()
                    .nth(width)
                    .map(|(idx, _)| idx)
                    .unwrap_or(word.len());
                let (head, tail) = word.split_at(split_at);
                out.push(head.to_string());
                if !tail.is_empty() {
                    pending.push_front(tail.to_string());
                }
                continue;
            }
            if len == 0 {
                line.push_str(&word);
                len = word_len;
            } else if len + 1 + word_len <= width {
                line.push(' ');
                line.push_str(&word);
                len += 1 + word_len;
            } else {
                out.push(std::mem::take(&mut line));
                len = 0;
                pending.push_front(word);
            }
        }
        if len > 0 {
            out.push(line);
        }
    }
    out
}

/// Alphabetical browse-sort key: case-insensitive title, untitled
/// sessions last, recency as the tie-breaker (matches the browse TUI
/// `F2` sort toggle).
pub fn browse_title_sort_key(
    title: Option<&str>,
) -> (u8, String) {
    match title.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => (0, t.to_lowercase()),
        None => (1, String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_basic_words() {
        let lines = wrap_display_text("hello world foo", 11);
        assert_eq!(lines, vec!["hello world", "foo"]);
    }

    #[test]
    fn wrap_preserves_paragraph_breaks() {
        let lines = wrap_display_text("a b\n\nc", 10);
        assert_eq!(lines, vec!["a b", "", "c"]);
    }

    #[test]
    fn wrap_hard_breaks_long_words() {
        let lines = wrap_display_text("abcdefghij", 4);
        assert_eq!(lines, vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn wrap_long_word_after_short_line() {
        let lines = wrap_display_text("x abcdefgh", 4);
        assert_eq!(lines, vec!["x", "abcd", "efgh"]);
    }

    #[test]
    fn wrap_collapse_whitespace_runs() {
        let lines = wrap_display_text("a   b\t\tc", 40);
        assert_eq!(lines, vec!["a b c"]);
    }

    #[test]
    fn wrap_counts_unicode_chars_not_bytes() {
        let lines = wrap_display_text("会话浏览 原始模式 升级", 9);
        assert_eq!(lines, vec!["会话浏览 原始模式", "升级"]);
    }

    #[test]
    fn wrap_empty_input() {
        let lines = wrap_display_text("", 10);
        assert_eq!(lines, vec![""]);
    }

    #[test]
    fn wrap_zero_width_degrades_to_one() {
        let lines = wrap_display_text("ab cd", 0);
        assert_eq!(lines, vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn sort_key_orders_untitled_last() {
        let a = browse_title_sort_key(Some("Zebra"));
        let b = browse_title_sort_key(Some("apple"));
        let c = browse_title_sort_key(None);
        let d = browse_title_sort_key(Some("   "));
        assert!(b < a); // case-insensitive
        assert!(a < c);
        assert_eq!(c, d); // blank titles count as untitled
    }
}
