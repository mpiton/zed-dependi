//! Shared helpers for span-aware JSON parsing (used by npm and php parsers).

use super::Span;

/// Pre-computed byte offsets of each line start in a source string.
///
/// Constructed in O(n); each `position` query is O(log n) via binary search.
#[derive(Debug)]
pub struct LineIndex {
    /// `offsets[i]` is the byte offset where line `i` starts.
    offsets: Vec<usize>,
}

impl LineIndex {
    /// Build a `LineIndex` from `content`. Always contains at least one entry (`0`).
    pub fn new(content: &str) -> Self {
        let mut offsets = Vec::with_capacity(content.len() / 32 + 1);
        offsets.push(0);
        for (i, byte) in content.bytes().enumerate() {
            if byte == b'\n' {
                offsets.push(i + 1);
            }
        }
        Self { offsets }
    }

    /// Convert a byte offset to a `(line, column)` pair, both 0-indexed.
    /// Columns are byte offsets within the line.
    /// Offsets past the end clamp to the last line.
    pub fn position(&self, byte_offset: usize) -> (u32, u32) {
        let line = match self.offsets.binary_search(&byte_offset) {
            Ok(exact) => exact,
            Err(insert_at) => insert_at.saturating_sub(1),
        };
        let col = byte_offset - self.offsets[line];
        (line as u32, col as u32)
    }
}

/// Convert a byte range `[start, end)` to a `Span` if it fits on a single line.
/// Returns `None` if the range straddles a line boundary.
pub fn span_to_span(line_index: &LineIndex, start: usize, end: usize) -> Option<Span> {
    let (line_start, col_start) = line_index.position(start);
    let (line_end, col_end) = line_index.position(end);
    if line_start != line_end {
        return None;
    }
    Some(Span {
        line: line_start,
        line_start: col_start,
        line_end: col_end,
    })
}

/// Strip the surrounding `"…"` from a JSON string's byte range.
/// `(start, end)` are the outer quote-inclusive bounds; the result is the
/// inner content bounds, suitable for `span_to_span`.
pub fn inner_string_span(start: usize, end: usize) -> (usize, usize) {
    (start + 1, end.saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_index_empty_content() {
        let idx = LineIndex::new("");
        assert_eq!(idx.position(0), (0, 0));
    }

    #[test]
    fn line_index_first_line() {
        let idx = LineIndex::new("hello\nworld");
        assert_eq!(idx.position(0), (0, 0));
        assert_eq!(idx.position(4), (0, 4));
    }

    #[test]
    fn line_index_after_newline() {
        let idx = LineIndex::new("hello\nworld");
        assert_eq!(idx.position(6), (1, 0));
        assert_eq!(idx.position(10), (1, 4));
    }

    #[test]
    fn line_index_three_lines() {
        let idx = LineIndex::new("a\nbb\nccc");
        assert_eq!(idx.position(0), (0, 0));
        assert_eq!(idx.position(2), (1, 0));
        assert_eq!(idx.position(5), (2, 0));
        assert_eq!(idx.position(7), (2, 2));
    }

    #[test]
    fn line_index_offset_past_end_clamps_to_last_line() {
        let idx = LineIndex::new("ab");
        let (line, _col) = idx.position(99);
        assert_eq!(line, 0);
    }

    #[test]
    fn line_index_multibyte_utf8_byte_columns() {
        // "é" is 2 bytes in UTF-8. Columns are byte offsets.
        let content = "ab\néd";
        let idx = LineIndex::new(content);
        assert_eq!(idx.position(3), (1, 0)); // 'é' start
        assert_eq!(idx.position(5), (1, 2)); // 'd' (after 2-byte 'é')
    }

    #[test]
    fn span_to_span_single_line() {
        let idx = LineIndex::new("hello world");
        let s = span_to_span(&idx, 6, 11).unwrap();
        assert_eq!(s.line, 0);
        assert_eq!(s.line_start, 6);
        assert_eq!(s.line_end, 11);
    }

    #[test]
    fn span_to_span_multi_line_returns_none() {
        let idx = LineIndex::new("hello\nworld");
        assert!(span_to_span(&idx, 4, 8).is_none());
    }

    #[test]
    fn inner_string_span_strips_quotes() {
        // For a JSON string `"abc"` at bytes 0..5, inner content is bytes 1..4.
        let (s, e) = inner_string_span(0, 5);
        assert_eq!((s, e), (1, 4));
    }

    #[test]
    fn inner_string_span_empty_string() {
        // Empty JSON string `""` at bytes 0..2, inner is 1..1.
        let (s, e) = inner_string_span(0, 2);
        assert_eq!((s, e), (1, 1));
    }
}
