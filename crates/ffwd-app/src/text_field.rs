//! A one-line text field with a cursor and select-all, for the palette,
//! the sidebar filter and the settings view. Positions are byte offsets
//! that always sit on character boundaries.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextField {
    text: String,
    cursor: usize,
    /// Everything selected (Ctrl+A): the next edit replaces it all.
    all: bool,
}

impl TextField {
    pub fn new(text: &str) -> TextField {
        TextField { text: text.to_string(), cursor: text.len(), all: false }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn all_selected(&self) -> bool {
        self.all && !self.text.is_empty()
    }

    fn take_selection(&mut self) -> bool {
        if std::mem::take(&mut self.all) && !self.text.is_empty() {
            self.text.clear();
            self.cursor = 0;
            return true;
        }
        false
    }

    pub fn insert(&mut self, s: &str) {
        self.take_selection();
        // One line: pasted newlines and tabs become spaces.
        let clean: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
        self.text.insert_str(self.cursor, &clean);
        self.cursor += clean.len();
    }

    pub fn backspace(&mut self) {
        if self.take_selection() || self.cursor == 0 {
            return;
        }
        let prev = self.text[..self.cursor].char_indices().next_back().map_or(0, |(i, _)| i);
        self.text.replace_range(prev..self.cursor, "");
        self.cursor = prev;
    }

    pub fn delete(&mut self) {
        if self.take_selection() || self.cursor >= self.text.len() {
            return;
        }
        let next = self.text[self.cursor..].chars().next().map_or(0, char::len_utf8);
        self.text.replace_range(self.cursor..self.cursor + next, "");
    }

    /// Ctrl+Backspace: removes the word before the cursor.
    pub fn delete_word(&mut self) {
        if self.take_selection() {
            return;
        }
        let before = &self.text[..self.cursor];
        let trimmed = before.trim_end();
        let start = trimmed.rfind(|c: char| c.is_whitespace() || "/\\:-_.".contains(c)).map_or(0, |i| i + 1);
        let start = if start == self.cursor && self.cursor > 0 { 0 } else { start };
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
    }

    pub fn left(&mut self) {
        self.all = false;
        self.cursor = self.text[..self.cursor].char_indices().next_back().map_or(0, |(i, _)| i);
    }

    pub fn right(&mut self) {
        self.all = false;
        if let Some(c) = self.text[self.cursor..].chars().next() {
            self.cursor += c.len_utf8();
        }
    }

    pub fn home(&mut self) {
        self.all = false;
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.all = false;
        self.cursor = self.text.len();
    }

    pub fn select_all(&mut self) {
        self.all = true;
        self.cursor = self.text.len();
    }

    /// The text Ctrl+C copies: everything (selected or not, the field is one line).
    pub fn copy_text(&self) -> &str {
        &self.text
    }

    /// Ctrl+X: copies everything and clears the field.
    pub fn cut(&mut self) -> String {
        self.all = false;
        self.cursor = 0;
        std::mem::take(&mut self.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing() {
        let mut f = TextField::new("main");
        f.left();
        f.left();
        f.insert("X");
        assert_eq!(f.text(), "maXin");
        f.home();
        f.delete();
        assert_eq!(f.text(), "aXin");
        f.end();
        f.backspace();
        assert_eq!(f.text(), "aXi");
        f.select_all();
        assert!(f.all_selected());
        f.insert("ü");
        assert_eq!(f.text(), "ü", "typing replaces a select-all");
        f.backspace();
        assert_eq!(f.text(), "");
        f.insert("E:\\Development\\kwil");
        f.delete_word();
        assert_eq!(f.text(), "E:\\Development\\");
        f.select_all();
        f.backspace();
        assert_eq!(f.text(), "");
        f.insert("a\nb");
        assert_eq!(f.text(), "a b", "one line");
    }

    #[test]
    fn multibyte_cursor_moves() {
        let mut f = TextField::new("ä日");
        f.left();
        assert_eq!(f.cursor(), "ä".len());
        f.backspace();
        assert_eq!(f.text(), "日");
        f.right();
        assert_eq!(f.cursor(), f.text().len());
    }
}
