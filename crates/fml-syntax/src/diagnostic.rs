//! Diagnostics and source positions.

use rowan::{TextRange, TextSize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
}

/// A problem found in the source, located by byte range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub range: TextRange,
    pub message: String,
}

impl Diagnostic {
    pub fn error(range: TextRange, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            range,
            message: message.into(),
        }
    }

    pub fn warning(range: TextRange, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            range,
            message: message.into(),
        }
    }
}

/// A position in the source: zero-based line, and zero-based column counted both in UTF-8 bytes
/// and in UTF-16 code units (the unit LSP and the Mapping Language debug trace use).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LineCol {
    pub line: u32,
    pub col: u32,
    pub col_utf16: u32,
}

/// Converts byte offsets to line and column.
#[derive(Debug, Clone)]
pub struct LineIndex {
    line_starts: Vec<TextSize>,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut line_starts = vec![TextSize::from(0)];
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'\n' => line_starts.push(TextSize::from((i + 1) as u32)),
                b'\r' if bytes.get(i + 1) != Some(&b'\n') => {
                    line_starts.push(TextSize::from((i + 1) as u32))
                }
                _ => {}
            }
            i += 1;
        }
        Self { line_starts }
    }

    /// The position of `offset` in `text`, which must be the text this index was built from.
    pub fn line_col(&self, text: &str, offset: TextSize) -> LineCol {
        let line = self
            .line_starts
            .partition_point(|&start| start <= offset)
            .saturating_sub(1);
        let start = self.line_starts[line];
        let prefix = &text[usize::from(start)..usize::from(offset)];
        LineCol {
            line: line as u32,
            col: prefix.len() as u32,
            col_utf16: prefix.encode_utf16().count() as u32,
        }
    }
    /// The offset of a zero-based line and UTF-16 column in `text`, which must be the text this
    /// index was built from. Positions past the end of a line or of the text are clamped.
    pub fn offset_utf16(&self, text: &str, line: u32, col_utf16: u32) -> TextSize {
        let Some(&start) = self.line_starts.get(line as usize) else {
            return TextSize::of(text);
        };
        let end = self
            .line_starts
            .get(line as usize + 1)
            .map_or(text.len(), |&next| usize::from(next));
        let mut offset = usize::from(start);
        let mut units = 0;
        for c in text[offset..end].chars() {
            if units >= col_utf16 || c == '\n' || c == '\r' {
                break;
            }
            units += c.len_utf16() as u32;
            offset += c.len_utf8();
        }
        TextSize::from(offset as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_and_columns() {
        let text = "ab\nцd\r\nx";
        let index = LineIndex::new(text);
        let at = |offset: u32| index.line_col(text, TextSize::from(offset));
        assert_eq!(
            at(0),
            LineCol {
                line: 0,
                col: 0,
                col_utf16: 0
            }
        );
        assert_eq!(
            at(5),
            LineCol {
                line: 1,
                col: 2,
                col_utf16: 1
            }
        );
        assert_eq!(
            at(8),
            LineCol {
                line: 2,
                col: 0,
                col_utf16: 0
            }
        );
    }

    #[test]
    fn offsets_from_utf16_positions() {
        let text = "ab\n𝄞d\r\nx";
        let index = LineIndex::new(text);
        let at = |line, col| u32::from(index.offset_utf16(text, line, col));
        assert_eq!(at(0, 1), 1);
        assert_eq!(at(1, 2), 7);
        assert_eq!(at(1, 3), 8);
        assert_eq!(at(1, 9), 8);
        assert_eq!(at(2, 0), 10);
        assert_eq!(at(7, 0), 11);
        for offset in [0u32, 1, 3, 7, 8, 10, 11] {
            let position = index.line_col(text, TextSize::from(offset));
            assert_eq!(at(position.line, position.col_utf16), offset);
        }
    }
}
