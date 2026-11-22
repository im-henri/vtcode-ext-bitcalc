//! Single-line text field with selection and undo.

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Line {
    chars: Vec<char>,
    pub cursor: usize,
    pub anchor: Option<usize>,
    undo: Vec<(Vec<char>, usize)>,
    redo: Vec<(Vec<char>, usize)>,
}

fn word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '\''
}

impl Line {
    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }
    pub fn chars(&self) -> &[char] {
        &self.chars
    }
    pub fn len(&self) -> usize {
        self.chars.len()
    }
    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }
    pub fn selection(&self) -> Option<(usize, usize)> {
        let a = self.anchor?;
        (a != self.cursor).then(|| (a.min(self.cursor), a.max(self.cursor)))
    }
    pub fn selected_text(&self) -> Option<String> {
        let (a, b) = self.selection()?;
        Some(self.chars[a..b].iter().collect())
    }
    fn snapshot(&mut self) {
        self.undo.push((self.chars.clone(), self.cursor));
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
    /// Replace the whole text (recorded for undo), cursor at the end.
    pub fn set(&mut self, text: &str) {
        if self.text() != text {
            self.snapshot();
        }
        self.chars = text.chars().collect();
        self.cursor = self.chars.len();
        self.anchor = None;
    }
    /// Replace the text without touching undo history.
    pub fn reset(&mut self, text: &str) {
        self.chars = text.chars().collect();
        self.cursor = self.chars.len();
        self.anchor = None;
    }
    pub fn clear(&mut self) {
        self.set("");
    }
    fn delete_selection(&mut self) -> bool {
        if let Some((a, b)) = self.selection() {
            self.chars.drain(a..b);
            self.cursor = a;
            self.anchor = None;
            true
        } else {
            self.anchor = None;
            false
        }
    }
    pub fn insert(&mut self, text: &str) {
        let text: Vec<char> = text
            .chars()
            .map(|c| if c == '\t' || c == '\n' || c == '\r' { ' ' } else { c })
            .filter(|c| !c.is_control())
            .collect();
        if text.is_empty() && self.selection().is_none() {
            return;
        }
        self.snapshot();
        self.delete_selection();
        let n = text.len();
        self.chars.splice(self.cursor..self.cursor, text);
        self.cursor += n;
    }
    pub fn backspace(&mut self, word: bool) {
        if self.selection().is_some() {
            self.snapshot();
            self.delete_selection();
        } else if self.cursor > 0 {
            self.snapshot();
            let to = if word { self.word_left(self.cursor) } else { self.cursor - 1 };
            self.chars.drain(to..self.cursor);
            self.cursor = to;
        }
    }
    pub fn delete(&mut self, word: bool) {
        if self.selection().is_some() {
            self.snapshot();
            self.delete_selection();
        } else if self.cursor < self.chars.len() {
            self.snapshot();
            let to = if word { self.word_right(self.cursor) } else { self.cursor + 1 };
            self.chars.drain(self.cursor..to);
        }
    }
    fn word_left(&self, mut i: usize) -> usize {
        while i > 0 && !word_char(self.chars[i - 1]) {
            i -= 1;
        }
        while i > 0 && word_char(self.chars[i - 1]) {
            i -= 1;
        }
        i
    }
    fn word_right(&self, mut i: usize) -> usize {
        let n = self.chars.len();
        while i < n && !word_char(self.chars[i]) {
            i += 1;
        }
        while i < n && word_char(self.chars[i]) {
            i += 1;
        }
        i
    }
    /// Move the caret; `select` extends the selection.
    pub fn move_to(&mut self, to: usize, select: bool) {
        if select {
            self.anchor.get_or_insert(self.cursor);
        } else {
            self.anchor = None;
        }
        self.cursor = to.min(self.chars.len());
    }
    pub fn left(&mut self, select: bool, word: bool) {
        if !select && !word && let Some((a, _)) = self.selection() {
            return self.move_to(a, false);
        }
        let to = if word { self.word_left(self.cursor) } else { self.cursor.saturating_sub(1) };
        self.move_to(to, select);
    }
    pub fn right(&mut self, select: bool, word: bool) {
        if !select && !word && let Some((_, b)) = self.selection() {
            return self.move_to(b, false);
        }
        let to = if word { self.word_right(self.cursor) } else { self.cursor + 1 };
        self.move_to(to, select);
    }
    pub fn select_all(&mut self) {
        self.anchor = Some(0);
        self.cursor = self.chars.len();
    }
    /// Select the word around `i` (double-click).
    pub fn select_word(&mut self, i: usize) {
        let i = i.min(self.chars.len());
        let mut a = i;
        while a > 0 && word_char(self.chars[a - 1]) {
            a -= 1;
        }
        let mut b = i;
        while b < self.chars.len() && word_char(self.chars[b]) {
            b += 1;
        }
        self.anchor = Some(a);
        self.cursor = b;
    }
    pub fn undo(&mut self) -> bool {
        let Some((chars, cursor)) = self.undo.pop() else { return false };
        self.redo.push((std::mem::replace(&mut self.chars, chars), self.cursor));
        self.cursor = cursor.min(self.chars.len());
        self.anchor = None;
        true
    }
    pub fn redo(&mut self) -> bool {
        let Some((chars, cursor)) = self.redo.pop() else { return false };
        self.undo.push((std::mem::replace(&mut self.chars, chars), self.cursor));
        self.cursor = cursor.min(self.chars.len());
        self.anchor = None;
        true
    }
}
