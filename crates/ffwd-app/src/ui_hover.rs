//! Pointer feedback: what is under the mouse (hover), what the mouse button
//! is holding down (press), and which cursor the window shows. Every
//! clickable thing answers the pointer in the next frame, and only the
//! element that changed is repainted (damage, no animation: performance
//! rules 2 and 5).

use super::*;

/// One interactive element.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hot {
    Top(TopAction),
    Search,
    /// The list title: all branches or only the current one.
    Scope,
    /// The comparison header's Swap (0) and Close (1).
    Compare(usize),
    /// The history header's Find button.
    HeadFind,
    /// The status bar's action button.
    Notice,
    /// A commit row (absolute index).
    Row(usize),
    /// A file of the selected commit.
    File(usize),
    Palette(usize),
    /// An entry of the settings view's hit list (row, choice).
    Setting(usize, usize),
    SettingsPage(usize),
    /// An entry of the agents panel's hit list.
    Agent(usize),
    /// An entry of the Local Changes view's hit list.
    Change(usize),
    /// An entry of the Files view's hit list.
    Files(usize),
    /// A row of the activity log.
    Activity(usize),
    /// A reflog entry.
    Reflog(usize),
    /// An element of the interactive rebase plan.
    Rebase(usize),
    /// An element of the find bar.
    Find(usize),
    /// A control in the diff bar.
    DiffBar(usize),
    /// A context menu item.
    Menu(usize),
    /// A dialog element.
    Dialog(usize),
}

/// The mouse cursor the window should show.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Cursor {
    #[default]
    Default,
    /// A button, tab, segment or link.
    Pointer,
    /// A text field.
    Text,
}

impl Ui {
    pub(super) fn palette_item_rect(&self, k: usize) -> Rect {
        let r = self.palette_rect(10);
        Rect::new(
            r.x + self.px(6.0),
            r.y + self.px(57.0) + k as i32 * self.px(34.0),
            r.w - self.px(12.0),
            self.px(34.0),
        )
    }

    pub(super) fn file_row_rect(&self, k: usize) -> Rect {
        let fr = self.files_rect();
        let rh = self.px(26.0);
        let d = self.lay.info;
        Rect::new(d.x + self.px(10.0), fr.y + (k as i32 - self.r.file_top as i32) * rh, d.w - self.px(20.0), rh)
    }

    /// The element under (x, y), topmost layer first.
    pub(super) fn hot_at(&self, x: i32, y: i32) -> Option<Hot> {
        if self.dialog.is_some() {
            return self.dialog_hit_at(x, y).map(Hot::Dialog);
        }
        if self.menu.is_some() {
            return self.menu_hit_at(x, y).map(Hot::Menu);
        }
        if let Some(i) = self.agent_hit_at(x, y) {
            return Some(Hot::Agent(i));
        }
        if self.agent_view.is_some() {
            return None;
        }
        if let Some(h) = self.settings_hit_at(x, y) {
            return Some(h);
        }
        if self.settings_view.is_some() {
            return None;
        }
        if let Some(p) = &self.palette {
            return (0..p.items.len()).find(|&k| self.palette_item_rect(k).contains(x, y)).map(Hot::Palette);
        }
        if let Some(&(_, a)) = self.top_buttons.iter().find(|(r, _)| r.contains(x, y)) {
            return Some(Hot::Top(a));
        }
        if self.notice_btn.contains(x, y) {
            return Some(Hot::Notice);
        }
        if self.r.rebase.is_some() {
            return self.rebase_hit_at(x, y).map(Hot::Rebase);
        }
        if self.r.reflog_view {
            return self.reflog_hit_at(x, y).map(Hot::Reflog);
        }
        if self.r.activity_view {
            return self.activity_hit_at(x, y).map(Hot::Activity);
        }
        if self.r.files_view {
            return self.files_hit_at(x, y).map(Hot::Files);
        }
        if self.r.changes_view {
            return self.change_hit_at(x, y).map(Hot::Change);
        }
        if self.search_rect.contains(x, y) {
            return Some(Hot::Search);
        }
        if self.r.search.is_none() && self.scope_rect.contains(x, y) {
            return Some(Hot::Scope);
        }
        if let Some(k) = self.compare_btns.iter().position(|b| b.contains(x, y)) {
            return Some(Hot::Compare(k));
        }
        if self.r.search.is_none() && self.head_find_rect.contains(x, y) {
            return Some(Hot::HeadFind);
        }
        if !self.r.changes_view && self.r.search.is_some() && self.lay.list_head.contains(x, y) {
            return self.search_hit_at(x, y).map(Hot::Find);
        }
        if !self.r.changes_view
            && let Some(i) = self.diff_bar_hit_at(x, y)
        {
            return Some(Hot::DiffBar(i));
        }
        let b = self.lay.list_body;
        if b.contains(x, y) {
            let row = self.r.top + ((y - b.y) / self.lay.row_h.max(1)) as usize;
            return (row < self.r.commits.len()).then_some(Hot::Row(row));
        }
        if self.files_rect().contains(x, y)
            && let Some(d) = &self.r.details
        {
            let k = self.r.file_top + ((y - self.files_rect().y) / self.px(26.0).max(1)) as usize;
            return (k < d.files.len()).then_some(Hot::File(k));
        }
        None
    }

    /// What to repaint when `h` changes state.
    pub(super) fn hot_rect(&self, h: Hot) -> Rect {
        match h {
            Hot::Top(a) => {
                self.top_buttons.iter().find(|(_, b)| *b == a).map_or(self.lay.top, |(r, _)| r.inset(-2, -2))
            }
            Hot::Search => self.search_rect.inset(-2, -2),
            Hot::Scope => self.scope_rect.inset(-2, -2),
            Hot::Compare(k) => self.compare_btns[k].inset(-2, -2),
            Hot::HeadFind => self.head_find_rect.inset(-2, -2),
            Hot::Notice => self.notice_btn.inset(-2, -2),
            Hot::Row(r) => self.row_rect(r).unwrap_or_default(),
            Hot::File(k) => self.file_row_rect(k),
            Hot::Palette(_) => self.palette_rect(10),
            Hot::Setting(..) | Hot::SettingsPage(_) => self.settings_rect(),
            Hot::Agent(_) => self.agents_rect(),
            Hot::Menu(_) => self.menu_rect(),
            Hot::Find(_) => self.lay.list_head,
            Hot::DiffBar(i) => self.diff_bar_hits.get(i).map_or(self.lay.details, |h| h.0.inset(-2, -2)),
            Hot::Dialog(_) => self.dialog_rect(),
            Hot::Change(i) => self.r.changes.hits.get(i).map_or(self.lay.list, |h| h.0.inset(-2, -2)),
            Hot::Files(i) => self.files_hit_rect(i),
            Hot::Activity(i) => self.activity_hit_rect(i),
            Hot::Reflog(i) => self.reflog_hit_rect(i),
            Hot::Rebase(i) => self.rebase_hit_rect(i),
        }
    }

    pub(super) fn is_hot(&self, h: Hot) -> bool {
        self.hot == Some(h)
    }

    pub(super) fn is_pressed(&self, h: Hot) -> bool {
        self.pressed == Some(h) && self.hot == Some(h)
    }

    /// Background and border of a control in its current state.
    pub(super) fn control_colors(&self, h: Hot, bg: u32, border: u32) -> (u32, u32) {
        if self.is_pressed(h) {
            (BG_PRESS, BORDER_HOVER)
        } else if self.is_hot(h) {
            (CONTROL_HOVER, BORDER_HOVER)
        } else {
            (bg, border)
        }
    }

    /// Updates hover after the pointer moved (or the content under it).
    pub(super) fn update_hot(&mut self, x: i32, y: i32) {
        self.pointer_at = Some((x, y));
        let h = self.hot_at(x, y);
        if h != self.hot {
            for old in [self.hot, h].into_iter().flatten() {
                let r = self.hot_rect(old);
                self.damage.rects.push(r);
            }
            self.hot = h;
        }
        self.cursor = match h {
            None => Cursor::Default,
            Some(Hot::Search) => Cursor::Text,
            Some(Hot::Row(_) | Hot::File(_)) => Cursor::Default,
            Some(Hot::Agent(i)) if self.agent_hit_is_field(i) => Cursor::Text,
            Some(Hot::Change(i)) if self.change_hit_is_text(i) => Cursor::Text,
            Some(Hot::Files(i)) if self.files_hit_is_text(i) => Cursor::Text,
            Some(Hot::Find(i)) if self.search_hit_is_field(i) => Cursor::Text,
            Some(Hot::Setting(row, _)) if self.setting_is_text(row) => Cursor::Text,
            Some(_) => Cursor::Pointer,
        };
        // Tabs, the sidebar and the caption buttons keep their own hover.
        if self.cursor == Cursor::Default
            && (self.tab_at(x, y).is_some() || self.caption_button_at(x, y).is_some_and(|b| b != 1))
        {
            self.cursor = Cursor::Pointer;
        }
    }

    /// After a scroll or a layout change the element under a resting
    /// pointer changes too.
    pub(super) fn refresh_hot(&mut self) {
        if let Some((x, y)) = self.pointer_at {
            self.update_hot(x, y);
        }
    }

    pub(super) fn clear_hot(&mut self) {
        self.pointer_at = None;
        if let Some(h) = self.hot.take() {
            let r = self.hot_rect(h);
            self.damage.rects.push(r);
        }
        self.cursor = Cursor::Default;
    }

    /// The mouse button went down on (x, y).
    pub(super) fn press_at(&mut self, x: i32, y: i32) {
        self.update_hot(x, y);
        self.pressed = self.hot;
        if let Some(h) = self.pressed {
            let r = self.hot_rect(h);
            self.damage.rects.push(r);
        }
    }

    pub(super) fn release_press(&mut self) {
        if let Some(h) = self.pressed.take() {
            let r = self.hot_rect(h);
            self.damage.rects.push(r);
        }
    }

    pub fn cursor(&self) -> Cursor {
        self.cursor
    }
}
