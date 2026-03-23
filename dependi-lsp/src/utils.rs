//! Common utility functions used across the LSP implementation.

use core::fmt::{self, Write as _};

/// Returns an <code>[fmt::Display] + [fmt::Debug]</code> implementation
/// which truncates the given string to a maximum character count with ellipsis.
///
/// This function properly handles UTF-8 strings by counting characters,
/// not bytes. If the string needs truncation, an ellipsis ("...") is
/// appended and counted toward the maximum length.
///
/// # Arguments
///
/// * `s` - The string to truncate
/// * `max_chars` - Maximum number of characters to display (including ellipsis)
///
/// # Returns
///
/// The truncated string, or the original string if it fits within `max_chars`.
#[must_use = "returns a type implementing Display and Debug, which does not have any effects unless they are used"]
pub fn fmt_truncate_string(s: &str, max_chars: usize) -> impl fmt::Display + fmt::Debug {
    fmt::from_fn(move |f| {
        let char_count = s.chars().count();
        if char_count <= max_chars {
            return f.write_str(s);
        }

        // When max_chars < 3, we can't fit the ellipsis ("...")
        // so just truncate to max_chars characters
        if max_chars < 3 {
            return s.chars().take(max_chars).try_for_each(|c| f.write_char(c));
        }

        let keep_chars = max_chars.saturating_sub(3);
        s.chars()
            .take(keep_chars)
            .try_for_each(|c| f.write_char(c))?;
        write!(f, "...")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn truncate_string(s: &str, max_chars: usize) -> String {
        fmt_truncate_string(s, max_chars).to_string()
    }

    #[test]
    fn test_no_truncation_needed() {
        assert_eq!(truncate_string("hello", 10), "hello");
        assert_eq!(truncate_string("", 10), "");
        assert_eq!(truncate_string("hello", 5), "hello");
    }

    #[test]
    fn test_ascii_truncation() {
        assert_eq!(truncate_string("hello world", 8), "hello...");
        assert_eq!(truncate_string("abcdefghij", 7), "abcd...");
    }

    #[test]
    fn test_edge_cases() {
        assert_eq!(truncate_string("hello", 3), "..."); // Exactly 3: ellipsis fits
        assert_eq!(truncate_string("hello", 4), "h...");
    }

    #[test]
    fn test_small_max_chars() {
        // max_chars < 3: truncate without ellipsis
        assert_eq!(truncate_string("hello", 2), "he");
        assert_eq!(truncate_string("ab", 1), "a");
        assert_eq!(truncate_string("a", 0), "");
        assert_eq!(truncate_string("hello world", 1), "h");
    }

    #[test]
    fn test_utf8_japanese() {
        // "日本語" = 3 characters, 9 bytes
        assert_eq!(truncate_string("日本語", 10), "日本語");
        assert_eq!(truncate_string("日本語", 3), "日本語");
        // "日本語test" = 7 characters
        // truncate to 6 means keep 3 + "..." = 6 total
        assert_eq!(truncate_string("日本語test", 6), "日本語...");
        // 7 chars <= 8, so no truncation needed
        assert_eq!(truncate_string("日本語test", 8), "日本語test");
        // 7 chars <= 7, so no truncation needed
        assert_eq!(truncate_string("日本語test", 7), "日本語test");
        // truncate to 5 means keep 2 + "..." = 5
        assert_eq!(truncate_string("日本語test", 5), "日本...");
    }

    #[test]
    fn test_utf8_emoji() {
        // "hello" = 5 characters, fits in 8
        assert_eq!(truncate_string("hello", 8), "hello");
        // "hello world" = 11 characters, truncate to 8 means keep 5 + "..."
        let emoji_str = "hello world";
        assert_eq!(truncate_string(emoji_str, 8), "hello...");
    }

    #[test]
    fn test_mixed_content() {
        assert_eq!(truncate_string("Hello 日本", 10), "Hello 日本");
        assert_eq!(truncate_string("Hello 日本語 world", 12), "Hello 日本語...");
    }
}
