//! A multi-line text field (the commit description). Positions are byte
//! offsets on character boundaries; `Enter` inserts a line break.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextArea {
    lines: Vec<String>,
    row: usize,
    col: usize,
    /// Everything selected (Ctrl+A): the next edit replaces it all.
    all: bool,
}

impl Default for TextArea {
    fn default() -> Self {
        TextArea { lines: vec![String::new()], row: 0, col: 0, all: false }
    }
}

impl TextArea {
    pub fn new(text: &str) -> TextArea {
        let lines: Vec<String> = text.split('\n').map(|l| l.trim_end_matches('\r').to_string()).collect();
        let row = lines.len() - 1;
        let col = lines[row].len();
        TextArea { lines, row, col, all: false }
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn is_empty(&self) -> bool {
        self.lines.iter().all(|l| l.is_empty())
    }

    pub fn cursor(&self) -> (usize, usize) {
        (self.row, self.col)
    }

    pub fn all_selected(&self) -> bool {
        self.all && !self.is_empty()
    }

    fn take_selection(&mut self) -> bool {
        if std::mem::take(&mut self.all) && !self.is_empty() {
            *self = TextArea::default();
            return true;
        }
        false
    }

    /// Inserts text; line breaks in it split lines, other control
    /// characters become spaces.
    pub fn insert(&mut self, s: &str) {
        self.take_selection();
        for (i, part) in s.split('\n').enumerate() {
            if i > 0 {
                self.newline();
            }
            let clean: String =
                part.trim_end_matches('\r').chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
            self.lines[self.row].insert_str(self.col, &clean);
            self.col += clean.len();
        }
    }

    pub fn newline(&mut self) {
        self.take_selection();
        let rest = self.lines[self.row].split_off(self.col);
        self.lines.insert(self.row + 1, rest);
        self.row += 1;
        self.col = 0;
    }

    pub fn backspace(&mut self) {
        if self.take_selection() {
            return;
        }
        if self.col > 0 {
            let line = &mut self.lines[self.row];
            let prev = line[..self.col].char_indices().next_back().map_or(0, |(i, _)| i);
            line.replace_range(prev..self.col, "");
            self.col = prev;
        } else if self.row > 0 {
            let line = self.lines.remove(self.row);
            self.row -= 1;
            self.col = self.lines[self.row].len();
            self.lines[self.row].push_str(&line);
        }
    }

    pub fn delete(&mut self) {
        if self.take_selection() {
            return;
        }
        let len = self.lines[self.row].len();
        if self.col < len {
            let line = &mut self.lines[self.row];
            let next = line[self.col..].chars().next().map_or(0, char::len_utf8);
            line.replace_range(self.col..self.col + next, "");
        } else if self.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].push_str(&next);
        }
    }

    pub fn delete_word(&mut self) {
        if self.take_selection() {
            return;
        }
        if self.col == 0 {
            return self.backspace();
        }
        let line = &mut self.lines[self.row];
        let before = &line[..self.col];
        let trimmed = before.trim_end();
        let start = trimmed.rfind(|c: char| c.is_whitespace() || "/\\:-_.,".contains(c)).map_or(0, |i| i + 1);
        let start = if start == self.col { 0 } else { start };
        line.replace_range(start..self.col, "");
        self.col = start;
    }

    pub fn left(&mut self) {
        self.all = false;
        if self.col > 0 {
            self.col = self.lines[self.row][..self.col].char_indices().next_back().map_or(0, |(i, _)| i);
        } else if self.row > 0 {
            self.row -= 1;
            self.col = self.lines[self.row].len();
        }
    }

    pub fn right(&mut self) {
        self.all = false;
        if let Some(c) = self.lines[self.row][self.col..].chars().next() {
            self.col += c.len_utf8();
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = 0;
        }
    }

    /// Moves a line up or down, keeping the column where it fits.
    /// Returns false at the first or last line.
    pub fn vertical(&mut self, down: bool) -> bool {
        self.all = false;
        let target = if down { self.row + 1 } else { self.row.wrapping_sub(1) };
        if target >= self.lines.len() {
            return false;
        }
        self.row = target;
        let line = &self.lines[self.row];
        let mut c = self.col.min(line.len());
        while !line.is_char_boundary(c) {
            c -= 1;
        }
        self.col = c;
        true
    }

    pub fn home(&mut self) {
        self.all = false;
        self.col = 0;
    }

    pub fn end(&mut self) {
        self.all = false;
        self.col = self.lines[self.row].len();
    }

    pub fn select_all(&mut self) {
        self.all = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing_across_lines() {
        let mut t = TextArea::default();
        t.insert("Why:");
        t.newline();
        t.insert("- fast ü");
        assert_eq!(t.text(), "Why:\n- fast ü");
        t.home();
        t.backspace();
        assert_eq!(t.text(), "Why:- fast ü", "backspace at a line start joins lines");
        t.newline();
        assert!(t.vertical(false));
        t.end();
        t.delete();
        assert_eq!(t.text(), "Why:- fast ü");
        t.insert("\r\npasted\nlines");
        assert_eq!(t.lines().len(), 3);
        t.select_all();
        t.insert("x");
        assert_eq!(t.text(), "x");
        assert!(!t.vertical(true));
        t.delete_word();
        assert!(t.is_empty());
    }
}
