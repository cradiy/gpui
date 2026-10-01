use std::ops::Range;

/// A bounded UTF-8 snapshot of committed text around an input selection.
/// Preedit text is excluded and replaced by a collapsed cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurroundingText {
    /// Committed text, without embedded NUL bytes.
    pub text: String,
    /// Cursor offset within `text`, in UTF-8 bytes.
    pub cursor: usize,
    /// Selection anchor within `text`, in UTF-8 bytes.
    pub anchor: usize,
}

impl SurroundingText {
    /// Extract up to `max_bytes`, preserving the complete selection and UTF-8 boundaries.
    /// All offsets and `marked` refer to UTF-8 bytes in `text`. Returns `None` for
    /// invalid offsets, a selection exceeding the limit, or embedded NUL bytes.
    pub fn from_utf8(
        text: &str,
        cursor: usize,
        anchor: usize,
        marked: Option<Range<usize>>,
        max_bytes: usize,
    ) -> Option<Self> {
        if !text.is_char_boundary(cursor) || !text.is_char_boundary(anchor) {
            return None;
        }
        let (left, selection, right) = if let Some(marked) = marked {
            if marked.start > cursor.min(anchor) || marked.end < cursor.max(anchor) {
                return None;
            }
            (text.get(..marked.start)?, "", text.get(marked.end..)?)
        } else {
            let start = cursor.min(anchor);
            let end = cursor.max(anchor);
            (&text[..start], &text[start..end], &text[end..])
        };
        let remaining = max_bytes.checked_sub(selection.len())?;
        let right_len = right.len().min(remaining - left.len().min(remaining / 2));
        let left_len = left.len().min(remaining - right_len);
        let mut start = left.len() - left_len;
        while !left.is_char_boundary(start) {
            start += 1;
        }
        let mut end = right_len;
        while !right.is_char_boundary(end) {
            end -= 1;
        }
        let left = &left[start..];
        let mut result = String::with_capacity(left.len() + selection.len() + end);
        result.push_str(left);
        result.push_str(selection);
        result.push_str(&right[..end]);
        if result.contains('\0') {
            return None;
        }
        let (cursor, anchor) = if cursor < anchor {
            (left.len(), left.len() + selection.len())
        } else {
            (left.len() + selection.len(), left.len())
        };
        Some(Self {
            text: result,
            cursor,
            anchor,
        })
    }

    /// Convert an IME deletion around the selection from UTF-8 bytes to UTF-16
    /// code-unit counts. The selected text itself is excluded from both counts.
    /// Invalid boundaries or requests outside the snapshot return `None`.
    pub fn deletion_utf16(&self, before: usize, after: usize) -> Option<(usize, usize)> {
        let start = self.cursor.min(self.anchor);
        let end = self.cursor.max(self.anchor);
        let before = self.text.get(start.checked_sub(before)?..start)?;
        let after = self.text.get(end..end.checked_add(after)?)?;
        Some((before.encode_utf16().count(), after.encode_utf16().count()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surrounding_bounds_preserve_unicode_and_reversed_selection() {
        let prefix = "你😀".repeat(1000);
        let text = format!("{prefix}选中{}", "好🌍".repeat(1000));
        let snapshot = SurroundingText::from_utf8(
            &text,
            prefix.len(),
            prefix.len() + "选中".len(),
            None,
            4000,
        )
        .unwrap();
        assert!(snapshot.text.len() <= 4000);
        assert_eq!(&snapshot.text[snapshot.cursor..snapshot.anchor], "选中");
        assert!(snapshot.cursor > 0);
        assert!(snapshot.anchor < snapshot.text.len());
        assert!(SurroundingText::from_utf8(&text, 0, text.len(), None, 4000).is_none());

        let start = SurroundingText::from_utf8(&text, 0, 0, None, 4000).unwrap();
        assert!(start.text.len() > 3990);
        let end = SurroundingText::from_utf8(&text, text.len(), text.len(), None, 4000).unwrap();
        assert!(end.text.len() > 3990);
        assert_eq!(end.cursor, end.text.len());
    }

    #[test]
    fn surrounding_excludes_preedit_and_rejects_invalid_offsets() {
        let snapshot = SurroundingText::from_utf8("前ni😀后", 5, 9, Some(3..9), 4000).unwrap();
        assert_eq!(
            snapshot,
            SurroundingText {
                text: "前后".into(),
                cursor: 3,
                anchor: 3
            }
        );
        assert!(SurroundingText::from_utf8("前😀后", 4, 4, None, 4000).is_none());
        assert!(SurroundingText::from_utf8("前😀后", 0, 0, Some(3..7), 4000).is_none());
        assert!(SurroundingText::from_utf8("a\0b", 1, 1, None, 4000).is_none());
    }

    #[test]
    fn surrounding_deletion_uses_bytes_and_excludes_selection() {
        let snapshot = SurroundingText::from_utf8("前😀选中🌍后", 7, 13, None, 4000).unwrap();
        assert_eq!(snapshot.deletion_utf16(4, 4), Some((2, 2)));
        assert_eq!(snapshot.deletion_utf16(7, 7), Some((3, 3)));
        assert_eq!(snapshot.deletion_utf16(1, 0), None);
        assert_eq!(snapshot.deletion_utf16(0, 3), None);
        assert_eq!(snapshot.deletion_utf16(8, 0), None);
        assert_eq!(snapshot.deletion_utf16(0, usize::MAX), None);
    }
}
