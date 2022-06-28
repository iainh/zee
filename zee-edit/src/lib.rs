pub mod graphemes;
pub mod movement;
pub mod tree;

mod diff;

use ropey::{Rope, RopeBuilder, RopeSlice};
use std::{cmp, ops::Range};

pub use self::{
    diff::{DeleteOperation, OpaqueDiff},
    graphemes::{CharIndex, RopeExt, RopeGraphemes},
    movement::Direction,
};

pub const TAB_WIDTH: usize = 4;

trait RopeCursorExt {
    fn cursor_to_line(&self, cursor: &Cursor) -> usize;

    fn slice_cursor(&self, cursor: &Cursor) -> RopeSlice;
}

impl RopeCursorExt for Rope {
    fn cursor_to_line(&self, cursor: &Cursor) -> usize {
        self.char_to_line(cursor.range.start)
    }

    fn slice_cursor(&self, cursor: &Cursor) -> RopeSlice {
        self.slice(cursor.range.start..cursor.range.end)
    }
}

/// A `Cursor` represents a user cursor associated with a text buffer.
///
/// `Cursor`s consist of a location in a `Rope` and optionally a selection and
/// desired visual offset.
#[derive(Clone, Debug, PartialEq)]
pub struct Cursor {
    /// The cursor position represented as the index of the gap between two adjacent
    /// characters inside a `Rope`.
    ///
    /// For a rope of length len, the valid range is 0..=length. The position is
    /// aligned to extended grapheme clusters and will never index a gap inside
    /// a grapheme.
    range: Range<CharIndex>,
    /// The start of a selection if in select mode, ending at `range.start` or
    /// `range.end`, depending on direction. Aligned to extended grapheme
    /// clusters.
    selection: Option<CharIndex>,
    visual_horizontal_offset: Option<usize>,
}

impl Default for Cursor {
    fn default() -> Self {
        Self::new()
    }
}

impl Cursor {
    pub fn new() -> Self {
        Self {
            range: 0..0,
            selection: None,
            visual_horizontal_offset: None,
        }
    }

    pub fn with_range(range: Range<CharIndex>) -> Self {
        Self {
            range,
            ..Self::new()
        }
    }

    #[cfg(test)]
    pub fn end_of_buffer(text: &Rope) -> Self {
        Self {
            range: text.prev_grapheme_boundary(text.len_chars())..text.len_chars(),
            visual_horizontal_offset: None,
            selection: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.range.is_empty()
    }

    pub fn range(&self) -> Range<CharIndex> {
        self.range.clone()
    }

    pub fn selection(&self) -> Range<CharIndex> {
        match self.selection {
            Some(selection) if selection > self.range.start => self.range.start..selection,
            Some(selection) if selection < self.range.start => selection..self.range.start,
            _ => self.range.clone(),
        }
    }

    pub fn column_offset(&self, text: &Rope) -> usize {
        let char_line_start = text.line_to_char(text.cursor_to_line(self));
        graphemes::width(&text.slice(char_line_start..self.range.start))
    }

    pub fn reconcile(&mut self, new_text: &Rope, diff: &OpaqueDiff) {
        let OpaqueDiff {
            char_index,
            old_char_length,
            new_char_length,
            ..
        } = *diff;

        let modified_range = char_index..cmp::max(old_char_length, new_char_length);

        // The edit starts after the end of the cursor, nothing to do
        if modified_range.start >= self.range.end {
            return;
        }

        // The edit ends before the start of the cursor
        if modified_range.end <= self.range.start {
            let (start, end) = (self.range.start, self.range.end);
            if old_char_length > new_char_length {
                let length_change = old_char_length - new_char_length;
                self.range = start.saturating_sub(length_change)..end.saturating_sub(length_change);
            } else {
                let length_change = new_char_length - old_char_length;
                self.range = start + length_change..end + length_change;
            };
        }

        // Otherwise, the change overlaps with the cursor
        let grapheme_start =
            new_text.prev_grapheme_boundary(cmp::min(self.range.end, new_text.len_chars()));
        let grapheme_end = new_text.next_grapheme_boundary(grapheme_start);
        self.range = grapheme_start..grapheme_end
    }

    pub fn begin_selection(&mut self) {
        self.selection = Some(self.range.start)
    }

    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    pub fn select_all(&mut self, text: &Rope) {
        movement::move_to_start_of_buffer(text, self);
        self.selection = Some(text.len_chars());
    }

    // Editing

    pub fn insert_char(&mut self, text: &mut Rope, character: char) -> OpaqueDiff {
        self.clear_selection();
        text.insert_char(self.range.start, character);
        OpaqueDiff::new(
            text.char_to_byte(self.range.start),
            0,
            character.len_utf8(),
            self.range.start,
            0,
            1,
        )
    }

    pub fn insert_chars(
        &mut self,
        text: &mut Rope,
        characters: impl IntoIterator<Item = char>,
    ) -> OpaqueDiff {
        self.insert_chars_at_index(text, self.range.start, characters)
    }

    pub fn prepend_chars(
        &mut self,
        text: &mut Rope,
        characters: impl IntoIterator<Item = char>,
    ) -> OpaqueDiff {
        let line_start = text.line_to_char(text.char_to_line(self.range.start));
        self.insert_chars_at_index(text, line_start, characters)
    }

    fn prepend_selection(
        &mut self,
        text: &mut Rope,
        characters: impl IntoIterator<Item = char>,
    ) -> OpaqueDiff {
        let first_line_idx = text.char_to_line(self.selection().start);
        let last_line_idx = text.char_to_line(self.selection().end);

        let end_of_last_line_idx = text.line_to_char(last_line_idx + 1) - 1;

        let old_byte_length =
            text.char_to_byte(end_of_last_line_idx) - text.line_to_byte(first_line_idx);

        let old_char_length = end_of_last_line_idx - text.line_to_char(first_line_idx);

        let mut prefix = RopeBuilder::new();
        characters
            .into_iter()
            .for_each(|c| prefix.append(&c.to_string()));

        let prefix = prefix.finish();

        let mut num_bytes = 0;
        let mut num_chars = 0;

        for line_idx in first_line_idx..=last_line_idx {
            let index = text.line_to_char(line_idx);
            let (nbytes, nchars) = self.internal_insert_chars_at_index(text, index, prefix.chars());

            num_bytes += nbytes;
            num_chars += nchars;
        }

        if let Some(selection) = self.selection {
            if self.selection().end == self.range.start {
                // Forward selection direction
                movement::move_horizontally(text, self, Direction::Forward, num_chars - 1);
                self.selection = Some(selection + num_chars / (last_line_idx - first_line_idx));
            } else {
                // 'Backwards' selection direction
                movement::move_horizontally(
                    text,
                    self,
                    Direction::Forward,
                    num_chars / (last_line_idx - first_line_idx) - 1,
                );
                self.selection = Some(selection + num_chars);
            }
        }

        let byte_index_of_first_line = text.char_to_byte(text.line_to_char(first_line_idx));

        OpaqueDiff::new(
            byte_index_of_first_line,
            old_byte_length,
            old_byte_length + num_bytes,
            text.line_to_char(first_line_idx),
            old_char_length,
            old_char_length + num_chars,
        )
    }

    fn internal_insert_chars_at_index(
        &mut self,
        text: &mut Rope,
        index: usize,
        characters: impl IntoIterator<Item = char>,
    ) -> (usize, usize) {
        let mut num_bytes = 0;
        let mut num_chars = 0;

        characters
            .into_iter()
            .enumerate()
            .for_each(|(offset, character)| {
                text.insert_char(index + offset, character);
                num_bytes += character.len_utf8();
                num_chars += 1;
            });
        (num_bytes, num_chars)
    }

    fn insert_chars_at_index(
        &mut self,
        text: &mut Rope,
        index: usize,
        characters: impl IntoIterator<Item = char>,
    ) -> OpaqueDiff {
        if self.selection.is_some() {
            self.prepend_selection(text, characters)
        } else {
            let (num_bytes, num_chars) =
                self.internal_insert_chars_at_index(text, index, characters);

            OpaqueDiff::new(
                text.char_to_byte(self.range.start),
                0,
                num_bytes,
                index,
                0,
                num_chars,
            )
        }
    }

    pub fn unindent(&mut self, text: &mut Rope) -> DeleteOperation {
        if self.selection.is_some() {
            self.unindent_selection(text)
        } else {
            let line_start = text.line_to_char(text.char_to_line(self.range.start));
            let count = self.length_of_leading_whitespace(text, line_start);

            self.delete_forward_from_index(text, line_start, count)
        }
    }

    fn unindent_selection(&mut self, text: &mut Rope) -> DeleteOperation {
        let first_line_idx = text.char_to_line(self.selection().start);
        let last_line_idx = text.char_to_line(self.selection().end);

        let initial_grapheme_start = text.line_to_char(first_line_idx);
        let initial_grapheme_end = text.line_to_char(last_line_idx + 1) - 1;

        let initial_byte_start = text.char_to_byte(initial_grapheme_start);
        let initial_byte_end = text.char_to_byte(initial_grapheme_end);

        let initial_byte_length = initial_byte_end - initial_byte_start;
        let initial_char_length = initial_grapheme_end - initial_grapheme_start;

        let mut total_chars_removed = 0;

        for line_idx in first_line_idx..=last_line_idx {
            let char_idx = text.line_to_char(line_idx);
            let line_start = text.line_to_char(text.char_to_line(char_idx));
            let length = self.length_of_leading_whitespace(text, line_start);

            text.remove(char_idx..char_idx + length);
            total_chars_removed += length;
        }

        // Start of the first line affected
        let final_grapheme_start = text.line_to_char(first_line_idx);
        // End of the last line affected
        let final_grapheme_end = text.line_to_char(last_line_idx + 1) - 1;
        let deleted: Rope = text.slice(final_grapheme_start..final_grapheme_end).into();

        let byte_range =
            text.char_to_byte(final_grapheme_start)..text.char_to_byte(final_grapheme_end);

        let diff = OpaqueDiff::new(
            byte_range.start,
            initial_byte_length,
            byte_range.end - byte_range.start,
            final_grapheme_start,
            initial_char_length,
            final_grapheme_end - final_grapheme_start,
        );

        if let Some(selection) = self.selection {
            let head_move_count = total_chars_removed / (last_line_idx - first_line_idx);
            if self.selection().end == self.range.start {
                // Forward selection direction
                let cursor_char_idx = self.range.start;
                let start_of_cursor_line = text.line_to_char(text.cursor_to_line(self));

                let move_distance =
                    cmp::min(total_chars_removed, cursor_char_idx - start_of_cursor_line);
                movement::move_horizontally(text, self, Direction::Backward, move_distance);

                if selection >= head_move_count {
                    self.selection = Some(selection - head_move_count);
                }
            } else {
                // 'Backwards' selection direction
                let cursor_line_whitespace = self
                    .length_of_leading_whitespace(text, text.char_to_line(self.selection().start));

                if cursor_line_whitespace >= head_move_count {
                    movement::move_horizontally(text, self, Direction::Backward, head_move_count);
                }

                if selection >= total_chars_removed {
                    self.selection = Some(selection - total_chars_removed);
                }
            }
        }

        DeleteOperation { diff, deleted }
    }

    fn length_of_leading_whitespace(&self, text: &mut Rope, line_start: usize) -> usize {
        match text.get_char(line_start) {
            Some('\t') => 1,
            Some(_) => match text.get_slice(line_start..line_start + TAB_WIDTH) {
                Some(leading_chars) => leading_chars
                    .chars()
                    .into_iter()
                    .position(|c| c != ' ')
                    .unwrap_or(TAB_WIDTH),
                None => 0,
            },
            None => 0,
        }
    }

    fn delete_forward_from_index(
        &mut self,
        text: &mut Rope,
        index: usize,
        length: usize,
    ) -> DeleteOperation {
        if text.len_chars() == 0 || text.len_chars() == index || length == 0 {
            return DeleteOperation::empty();
        }

        let byte_range = text.char_to_byte(index)..text.char_to_byte(index + length);
        let diff = OpaqueDiff::new(
            byte_range.start,
            byte_range.end - byte_range.start,
            0,
            index,
            length,
            0,
        );

        text.remove(index..index + length);

        let grapheme_start = index;
        let grapheme_end = text.next_grapheme_boundary(index);
        let deleted = text.slice(grapheme_start..grapheme_end).into();

        *self = Cursor::with_range(grapheme_start..grapheme_end);

        DeleteOperation { diff, deleted }
    }

    pub fn delete_forward(&mut self, text: &mut Rope) -> DeleteOperation {
        let index = self.range.start;
        let length = self.range.end - self.range.start;
        self.delete_forward_from_index(text, index, length)
    }

    pub fn delete_backward(&mut self, text: &mut Rope) -> DeleteOperation {
        if self.range.start > 0 {
            movement::move_horizontally(text, self, Direction::Backward, 1);
            self.delete_forward(text)
        } else {
            DeleteOperation::empty()
        }
    }

    pub fn delete_line(&mut self, text: &mut Rope) -> DeleteOperation {
        if text.len_chars() == 0 {
            return DeleteOperation::empty();
        }

        // Delete line
        let line_index = text.char_to_line(self.range.start);
        let delete_range_start = text.line_to_char(line_index);
        let delete_range_end = text.line_to_char(line_index + 1);
        let deleted = text.slice(delete_range_start..delete_range_end).into();
        let diff = OpaqueDiff::new(
            text.char_to_byte(delete_range_start),
            text.char_to_byte(delete_range_end) - text.char_to_byte(delete_range_start),
            0,
            delete_range_start,
            delete_range_end - delete_range_start,
            0,
        );
        text.remove(delete_range_start..delete_range_end);

        // Update cursor position
        let grapheme_start =
            text.line_to_char(cmp::min(line_index, text.len_lines().saturating_sub(2)));
        let grapheme_end = text.next_grapheme_boundary(grapheme_start);

        *self = Cursor::with_range(grapheme_start..grapheme_end);

        DeleteOperation { diff, deleted }
    }

    pub fn delete_selection(&mut self, text: &mut Rope) -> DeleteOperation {
        if text.len_chars() == 0 {
            return DeleteOperation::empty();
        }

        // Delete selection
        let selection = self.selection();
        let deleted = text.slice(selection.start..selection.end).into();
        let diff = OpaqueDiff::new(
            text.char_to_byte(selection.start),
            text.char_to_byte(selection.end) - text.char_to_byte(selection.start),
            0,
            selection.start,
            selection.end - selection.start,
            0,
        );
        text.remove(selection.start..selection.end);

        // Update cursor position
        let grapheme_start = cmp::min(
            self.range.start,
            text.prev_grapheme_boundary(text.len_chars()),
        );
        let grapheme_end = text.next_grapheme_boundary(grapheme_start);

        *self = Cursor::with_range(grapheme_start..grapheme_end);

        DeleteOperation { diff, deleted }
    }

    pub fn sync(&mut self, current_text: &Rope, new_text: &Rope) {
        let current_line = current_text.char_to_line(self.range.start);
        let current_line_offset = self.range.start - current_text.line_to_char(current_line);

        let new_line = cmp::min(current_line, new_text.len_lines().saturating_sub(1));
        let new_line_offset = cmp::min(
            current_line_offset,
            new_text.line(new_line).len_chars().saturating_sub(1),
        );
        let grapheme_end =
            new_text.next_grapheme_boundary(new_text.line_to_char(new_line) + new_line_offset);
        let grapheme_start = new_text.prev_grapheme_boundary(grapheme_end);

        *self = Cursor::with_range(grapheme_start..grapheme_end);
    }
}

#[cfg(test)]
mod tests {
    use ropey::Rope;

    use crate::movement::move_backward_paragraph;

    use super::*;

    fn text_with_cursor(text: impl Into<Rope>) -> (Rope, Cursor) {
        let text = text.into();
        let mut cursor = Cursor::new();
        movement::move_horizontally(&text, &mut cursor, Direction::Backward, 1);
        (text, cursor)
    }

    #[test]
    fn sync_with_empty() {
        let current_text = Rope::from("Buy a milk goat\nAt the market\n");
        let new_text = Rope::from("");
        let mut cursor = Cursor::new();
        movement::move_horizontally(&current_text, &mut cursor, Direction::Forward, 4);
        cursor.sync(&current_text, &new_text);
        assert_eq!(Cursor::new(), cursor);
    }

    // Delete forward
    #[test]
    fn delete_forward_at_the_end() {
        let (mut text, mut cursor) = text_with_cursor(TEXT);
        let expected = text.clone();
        movement::move_to_end_of_buffer(&text, &mut cursor);
        cursor.delete_forward(&mut text);
        assert_eq!(expected, text);
    }

    #[test]
    fn delete_forward_empty_text() {
        let (mut text, mut cursor) = text_with_cursor("");
        cursor.delete_forward(&mut text);
        assert_eq!(cursor, Cursor::new());
    }

    #[test]
    fn delete_forward_at_the_begining() {
        let (mut text, mut cursor) = text_with_cursor("// Hello world!\n\n");
        let expected = Rope::from("Hello world!\n\n");
        cursor.delete_forward(&mut text);
        cursor.delete_forward(&mut text);
        cursor.delete_forward(&mut text);
        assert_eq!(expected, text);
    }

    #[test]
    fn delete_forward_from_index() {
        let (mut text, mut cursor) = text_with_cursor("// Hello world!\n\n");
        let expected = Rope::from("// Hello rld!\n\n");

        cursor.delete_forward_from_index(&mut text, 9, 2);
        assert_eq!(expected, text);
    }

    // Delete backward
    #[test]
    fn delete_backward_at_the_end() {
        let (mut text, mut cursor) = text_with_cursor("// Hello world!\n");
        movement::move_to_end_of_buffer(&text, &mut cursor);
        cursor.delete_backward(&mut text);
        assert_eq!(Rope::from("// Hello world!"), text);
        cursor.delete_backward(&mut text);
        assert_eq!(Rope::from("// Hello world"), text);
    }

    #[test]
    fn delete_backward_empty_text() {
        let (mut text, mut cursor) = text_with_cursor("");
        cursor.delete_backward(&mut text);
        assert_eq!(cursor, Cursor::new());
    }

    #[test]
    fn delete_backward_at_the_begining() {
        let (mut text, mut cursor) = text_with_cursor("// Hello world!\n");
        let expected = text.clone();
        cursor.delete_backward(&mut text);
        assert_eq!(expected, text);
    }

    #[test]
    fn unindent_selection_text_on_all_lines() {
        let (mut text, mut cursor) = text_with_cursor("\tline1\n\tline2\n\tline3\n");
        cursor.begin_selection();
        movement::move_to_end_of_buffer(&text, &mut cursor);
        movement::move_horizontally(&text, &mut cursor, Direction::Backward, 4);

        let expected = "line1\nline2\nline3\n";
        cursor.unindent_selection(&mut text);
        assert_eq!(expected, text);
    }

    #[test]
    fn unindent_selection_text_on_all_lines_bottom_up() {
        let (mut text, mut cursor) = text_with_cursor("\tline1\n\tline2\n\tline3\n");
        movement::move_to_end_of_buffer(&text, &mut cursor);
        cursor.begin_selection();
        movement::move_to_start_of_buffer(&text, &mut cursor);

        let expected = "line1\nline2\nline3\n";
        cursor.unindent_selection(&mut text);
        assert_eq!(expected, text);
    }

    #[test]
    fn unindent_selection_blank_last_line() {
        let (mut text, mut cursor) = text_with_cursor("\tline1\n\tline2\n");
        cursor.begin_selection();
        movement::move_to_end_of_buffer(&text, &mut cursor);
        // TODO: There is a bug where the cursor can't be at the end of the buffer that I haven't
        //       found the cause of yet.
        movement::move_horizontally(&text, &mut cursor, Direction::Backward, 2);

        let expected = "line1\nline2\n";
        cursor.unindent_selection(&mut text);
        assert_eq!(expected, text);
    }

    #[test]
    fn unindent_selection_blank_last_line_bottom_up() {
        let (mut text, mut cursor) = text_with_cursor("\tline1\n\tline2\n\n\n");
        movement::move_to_end_of_buffer(&text, &mut cursor);
        cursor.begin_selection();
        movement::move_to_start_of_buffer(&text, &mut cursor);

        let expected = "line1\nline2\n\n\n";
        cursor.unindent_selection(&mut text);
        assert_eq!(expected, text);
    }

    #[test]
    fn unindent_selection_blank_first_line() {
        let (mut text, mut cursor) = text_with_cursor("\n\tline1\nline2\n");
        cursor.begin_selection();
        movement::move_to_end_of_buffer(&text, &mut cursor);
        movement::move_horizontally(&text, &mut cursor, Direction::Backward, 2);

        let expected = "\nline1\nline2\n";
        cursor.unindent_selection(&mut text);

        assert_eq!(expected, text);

        assert_eq!(cursor.selection().start, 0);
        assert_eq!(cursor.selection().end, text.len_chars() - 2);
    }

    #[test]
    fn unindent_selection_blank_first_line_bottom_up() {
        let (mut text, mut cursor) = text_with_cursor("\n\t\tline1\n\tline2\n");
        movement::move_to_end_of_buffer(&text, &mut cursor);
        cursor.begin_selection();
        movement::move_to_start_of_buffer(&text, &mut cursor);

        cursor.unindent_selection(&mut text);
        cursor.unindent_selection(&mut text);

        let expected = "\nline1\nline2\n";
        assert_eq!(expected, text);

        assert_eq!(cursor.selection().start, 0);
        assert_eq!(cursor.selection().end, text.len_chars());
    }

    // Leading whitespace calculation

    #[test]
    fn length_of_leading_whitespace_spaces() {
        //    fn length_of_leading_whitespace(&mut self, text: &mut Rope, line_start: usize) -> usize {
        let (mut text, cursor) = text_with_cursor("    // Hello world!\n\n");
        let expected = 4;
        let result = cursor.length_of_leading_whitespace(&mut text, 0);
        assert_eq!(expected, result);
    }

    #[test]
    fn length_of_leading_whitespace_tab() {
        // fn length_of_leading_whitespace(&mut self, text: &mut Rope, line_start: usize) -> usize {
        let (mut text, cursor) = text_with_cursor("\t// Hello world!\n\n");
        let expected = 1;
        let result = cursor.length_of_leading_whitespace(&mut text, 0);
        assert_eq!(expected, result);
    }

    #[test]
    fn length_of_leading_whitespace_mixed() {
        let (mut text, cursor) = text_with_cursor("  \t// Hello world!\n\n");
        let expected = 2;
        let result = cursor.length_of_leading_whitespace(&mut text, 0);
        assert_eq!(expected, result);
    }

    const TEXT: &str = r#"
Basic Latin
    ! " # $ % & ' ( ) *+,-./012ABCDEFGHI` a m  t u v z { | } ~
CJK
    豈 更 車 Ⅷ
"#;
}
