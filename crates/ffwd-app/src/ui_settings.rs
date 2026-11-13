//! The settings view (gear button, Ctrl+,). Every change is saved to
//! `settings.toml` at once, on a worker; the file stays hand-editable and
//! keeps its comments.

use super::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Row {
    Layout,
    Updates,
    Channel,
    Roots,
    AiEndpoint,
    AiModel,
    OpenFile,
    CheckUpdates,
}

const ROWS: [Row; 8] = [
    Row::Layout,
    Row::Updates,
    Row::Channel,
    Row::Roots,
    Row::AiEndpoint,
    Row::AiModel,
    Row::OpenFile,
    Row::CheckUpdates,
];

impl Row {
    fn label(self) -> &'static str {
        match self {
            Row::Layout => "Commit details",
            Row::Updates => "Automatic updates",
            Row::Channel => "Update channel",
            Row::Roots => "Repository folders",
            Row::AiEndpoint => "AI endpoint",
            Row::AiModel => "AI model",
            Row::OpenFile => "Settings file",
            Row::CheckUpdates => "Updates",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Row::Layout => "Where the selected commit's details and diff go",
            Row::Updates => "Installed copies check once a day",
            Row::Channel => "Automatic follows the running version",
            Row::Roots => "Folders searched for repositories in Open repository (; between folders)",
            Row::AiEndpoint => "OpenAI-compatible URL; local servers first (Ollama, LM Studio, llama.cpp)",
            Row::AiModel => "Model name the endpoint expects",
            Row::OpenFile => "Every setting, as text",
            Row::CheckUpdates => "",
        }
    }

    fn key(self) -> Option<&'static str> {
        match self {
            Row::Roots => Some("repos.roots"),
            Row::AiEndpoint => Some("ai.endpoint"),
            Row::AiModel => Some("ai.model"),
            _ => None,
        }
    }

    fn placeholder(self) -> &'static str {
        match self {
            Row::Roots => "Folders next to the open repositories",
            Row::AiEndpoint => "Off · e.g. http://localhost:11434/v1",
            Row::AiModel => "e.g. qwen2.5-coder:14b",
            _ => "",
        }
    }

    fn choices(self) -> &'static [&'static str] {
        match self {
            Row::Layout => &["Below the history", "Beside the history"],
            Row::Updates => &["On", "Off"],
            Row::Channel => &["Automatic", "Stable", "Beta"],
            _ => &[],
        }
    }
}

pub struct SettingsView {
    sel: usize,
    /// The text row being edited and its field.
    edit: Option<(usize, TextField)>,
    /// Option hit areas from the last paint: (row, choice, rect).
    hits: Vec<(usize, usize, Rect)>,
}

impl Ui {
    pub fn open_settings(&mut self) {
        self.palette = None;
        self.settings_view = Some(SettingsView { sel: 0, edit: None, hits: Vec::new() });
        let r = self.settings_rect();
        self.damage.rects.push(r);
    }

    pub(super) fn close_settings(&mut self) {
        if self.settings_view.take().is_some() {
            let r = self.settings_rect();
            self.damage.rects.push(r);
        }
    }

    pub(super) fn settings_rect(&self) -> Rect {
        let w = self.px(760.0).min(self.w as i32 - self.px(60.0));
        let h = (self.px(70.0) + ROWS.len() as i32 * self.px(54.0) + self.px(20.0)).min(self.h as i32 - self.px(60.0));
        Rect::new((self.w as i32 - w) / 2, (self.h as i32 - h) / 2, w, h)
    }

    fn choice_now(&self, row: Row) -> usize {
        match row {
            Row::Layout => (self.layout == DetailsLayout::Beside) as usize,
            Row::Updates => (self.settings.get("update.check") == Some("false")) as usize,
            Row::Channel => match self.settings.get("update.channel") {
                Some("stable") => 1,
                Some("beta") => 2,
                _ => 0,
            },
            _ => 0,
        }
    }

    fn set_choice(&mut self, row: Row, i: usize) {
        match row {
            Row::Layout => self.set_layout(if i == 1 { DetailsLayout::Beside } else { DetailsLayout::Below }),
            Row::Updates => self.settings.set_string("update.check", if i == 1 { "false" } else { "true" }),
            Row::Channel => self.settings.set_string("update.channel", ["auto", "stable", "beta"][i.min(2)]),
            _ => {}
        }
        let r = self.settings_rect();
        self.damage.rects.push(r);
    }

    fn settings_act(&mut self, i: usize) {
        let row = ROWS[i];
        match row {
            Row::OpenFile => open_settings_file(),
            Row::CheckUpdates => {
                self.close_settings();
                self.request_update_check();
            }
            r if r.key().is_some() => {
                let cur = self.settings.get(r.key().unwrap_or_default()).unwrap_or("").to_string();
                if let Some(v) = &mut self.settings_view {
                    v.edit = Some((i, TextField::new(&cur)));
                }
            }
            r => {
                let n = r.choices().len().max(1);
                let next = (self.choice_now(r) + 1) % n;
                self.set_choice(r, next);
            }
        }
        let r = self.settings_rect();
        self.damage.rects.push(r);
    }

    fn commit_edit(&mut self) {
        let Some(v) = &mut self.settings_view else { return };
        if let Some((i, f)) = v.edit.take()
            && let Some(key) = ROWS[i].key()
        {
            let text = f.text().trim().to_string();
            self.settings.set_string(key, &text);
            if key == "repos.roots" {
                self.scan_repos();
            }
        }
    }

    /// All keys go to the settings view while it is open.
    pub(super) fn settings_key(&mut self, k: Key) {
        let Some(v) = &mut self.settings_view else { return };
        if let Some((_, f)) = &mut v.edit {
            match k {
                Key::Enter | Key::Tab => self.commit_edit(),
                Key::Esc => v.edit = None,
                Key::Char(c) => f.insert(c.encode_utf8(&mut [0; 4])),
                Key::Backspace => f.backspace(),
                Key::CtrlBackspace => f.delete_word(),
                Key::Delete => f.delete(),
                Key::Left => f.left(),
                Key::Right => f.right(),
                Key::Home => f.home(),
                Key::End => f.end(),
                Key::Ctrl('a') => f.select_all(),
                Key::Ctrl('c') => crate::platform::set_clipboard(f.copy_text()),
                Key::Ctrl('v') => {
                    if let Some(t) = crate::platform::clipboard() {
                        f.insert(&t);
                    }
                }
                _ => {}
            }
            let r = self.settings_rect();
            self.damage.rects.push(r);
            return;
        }
        let sel = v.sel;
        match k {
            Key::Esc | Key::Ctrl(',') => self.close_settings(),
            Key::Up | Key::BackTab => v.sel = sel.saturating_sub(1),
            Key::Down | Key::Tab => v.sel = (sel + 1).min(ROWS.len() - 1),
            Key::Enter | Key::Char(' ') => self.settings_act(sel),
            Key::Left | Key::Right if !ROWS[sel].choices().is_empty() => {
                let n = ROWS[sel].choices().len();
                let cur = self.choice_now(ROWS[sel]);
                let next = if matches!(k, Key::Left) { (cur + n - 1) % n } else { (cur + 1) % n };
                self.set_choice(ROWS[sel], next);
            }
            Key::Char(c) if ROWS[sel].key().is_some() => {
                // Typing on a text row starts editing it.
                self.settings_act(sel);
                if let Some(SettingsView { edit: Some((_, f)), .. }) = &mut self.settings_view {
                    f.select_all();
                    f.insert(c.encode_utf8(&mut [0; 4]));
                }
            }
            _ => {}
        }
        let r = self.settings_rect();
        self.damage.rects.push(r);
    }

    pub(super) fn settings_click(&mut self, x: i32, y: i32) {
        let r = self.settings_rect();
        if !r.contains(x, y) {
            self.commit_edit();
            self.close_settings();
            return;
        }
        let hit = self.settings_view.as_ref().and_then(|v| v.hits.iter().find(|h| h.2.contains(x, y)).copied());
        if let Some((row, choice, _)) = hit {
            self.commit_edit();
            if let Some(v) = &mut self.settings_view {
                v.sel = row;
            }
            if ROWS[row].choices().is_empty() {
                self.settings_act(row);
            } else {
                self.set_choice(ROWS[row], choice);
            }
        }
        self.damage.rects.push(r);
    }

    pub(super) fn paint_settings(&mut self, cv: &mut Canvas) {
        let Some(view) = self.settings_view.take() else { return };
        let r = self.settings_rect();
        let old = cv.set_clip(cv.clip().intersect(r));
        cv.round_border(r, self.pf(12.0), 0x343a45, BG_PANEL);
        let (ui, semi, mono) = (self.f.ui, self.f.semi, self.f.mono);
        let pad = self.pf(28.0);
        let x0 = r.x as f32 + pad;
        let title_y = r.y as f32 + self.pf(40.0);
        let e = self.te.draw(cv, semi, self.pf(18.0), x0, title_y, "Settings", TEXT, self.pf(200.0));
        let ver = format!("ffwd {}", env!("CARGO_PKG_VERSION"));
        self.te.draw(cv, mono, self.pf(12.0), e + self.pf(14.0), title_y, &ver, TEXT_4, self.pf(200.0));
        let hint = "Esc closes · saved at once";
        let hw = self.te.measure(ui, self.pf(11.5), hint);
        self.te.draw(cv, ui, self.pf(11.5), r.right() as f32 - pad - hw, title_y, hint, TEXT_4, hw + 2.0);
        let mut hits = Vec::new();
        let rh = self.px(54.0);
        let top = r.y + self.px(64.0);
        let ctrl_x = r.x + (r.w as f32 * 0.44) as i32;
        let ctrl_w = r.right() - self.px(24.0) - ctrl_x;
        for (i, row) in ROWS.iter().enumerate() {
            let y = top + i as i32 * rh;
            let rr = Rect::new(r.x + self.px(12.0), y, r.w - self.px(24.0), rh - self.px(4.0));
            if i == view.sel {
                cv.fill_round(rr, self.pf(8.0), BG_SELECT_DIM);
            }
            let cy = y as f32 + rh as f32 / 2.0 - self.pf(2.0);
            self.te.draw(
                cv,
                semi,
                self.pf(13.0),
                x0,
                cy - self.pf(2.0),
                row.label(),
                TEXT,
                (ctrl_x - r.x) as f32 - pad,
            );
            self.te.draw(
                cv,
                ui,
                self.pf(11.5),
                x0,
                cy + self.pf(14.0),
                row.hint(),
                TEXT_3,
                (ctrl_x - r.x) as f32 - pad - self.pf(8.0),
            );
            let ctrl = Rect::new(ctrl_x, (cy - self.pf(15.0)) as i32, ctrl_w, self.px(30.0));
            let base = (cy + self.pf(4.5)).round();
            match *row {
                Row::OpenFile | Row::CheckUpdates => {
                    let label = if *row == Row::OpenFile { "Open settings.toml" } else { "Check for updates now" };
                    let w = self.te.measure(semi, self.pf(12.5), label) + self.pf(28.0);
                    let b = Rect::new(ctrl.x, ctrl.y, w as i32, ctrl.h);
                    cv.round_border(b, self.pf(8.0), BORDER, BG_BAR);
                    self.te.draw(cv, semi, self.pf(12.5), b.x as f32 + self.pf(14.0), base, label, TEXT, w);
                    hits.push((i, 0, b));
                }
                r2 if r2.key().is_some() => {
                    let editing = view.edit.as_ref().filter(|(j, _)| *j == i).map(|(_, f)| f.clone());
                    cv.round_border(ctrl, self.pf(7.0), if editing.is_some() { ACCENT } else { BORDER }, BG_INPUT);
                    let tx = ctrl.x as f32 + self.pf(12.0);
                    let maxw = ctrl.w as f32 - self.pf(24.0);
                    match editing {
                        Some(f) => {
                            if f.all_selected() {
                                let w = self.te.measure(ui, self.pf(12.5), f.text()).min(maxw);
                                cv.fill_round(
                                    Rect::new(
                                        tx as i32 - 2,
                                        ctrl.y + self.px(6.0),
                                        w as i32 + 4,
                                        ctrl.h - self.px(12.0),
                                    ),
                                    self.pf(3.0),
                                    0x2c4a78,
                                );
                            }
                            self.te.draw(cv, ui, self.pf(12.5), tx, base, f.text(), TEXT, maxw);
                            let cx = tx + self.te.measure(ui, self.pf(12.5), &f.text()[..f.cursor()]);
                            cv.fill(
                                Rect::new(
                                    cx as i32 + 1,
                                    ctrl.y + self.px(7.0),
                                    self.px(1.5).max(1),
                                    ctrl.h - self.px(14.0),
                                ),
                                ACCENT,
                            );
                        }
                        None => {
                            let v = self.settings.get(r2.key().unwrap_or_default()).unwrap_or("").to_string();
                            if v.is_empty() {
                                self.te.draw(cv, ui, self.pf(12.5), tx, base, r2.placeholder(), TEXT_4, maxw);
                            } else {
                                self.te.draw(cv, ui, self.pf(12.5), tx, base, &v, TEXT, maxw);
                            }
                        }
                    }
                    hits.push((i, 0, ctrl));
                }
                r2 => {
                    // Segmented control.
                    let choices = r2.choices();
                    let now = self.choice_now(r2);
                    let seg_w = ctrl.w / choices.len().max(1) as i32;
                    cv.round_border(ctrl, self.pf(7.0), BORDER, BG_INPUT);
                    for (c, name) in choices.iter().enumerate() {
                        let s = Rect::new(ctrl.x + c as i32 * seg_w, ctrl.y, seg_w, ctrl.h);
                        if c == now {
                            cv.fill_round(s.inset(self.px(3.0), self.px(3.0)), self.pf(5.0), 0x2a3346);
                        }
                        let tw = self.te.measure(if c == now { semi } else { ui }, self.pf(12.5), name);
                        let col = if c == now { TEXT } else { TEXT_3 };
                        self.te.draw(
                            cv,
                            if c == now { semi } else { ui },
                            self.pf(12.5),
                            s.x as f32 + (s.w as f32 - tw) / 2.0,
                            base,
                            name,
                            col,
                            s.w as f32,
                        );
                        hits.push((i, c, s));
                    }
                }
            }
        }
        cv.set_clip(old);
        self.settings_view = Some(SettingsView { hits, ..view });
    }
}

/// Opens settings.toml in the default editor (creating it with a short,
/// commented template first). Runs on a worker: file and process work stay
/// off the UI thread.
fn open_settings_file() {
    let Some(path) = crate::settings::path() else { return };
    std::thread::spawn(move || {
        if !path.exists() {
            if let Some(d) = path.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(
                &path,
                "# ffwd settings. ffwd keeps your comments when it changes a value.\n\n[ui]\n# layout = \"below\"   # or \"beside\"\n\n[update]\n# check = true\n# channel = \"auto\"   # \"stable\" or \"beta\"\n\n[repos]\n# roots = \"E:\\\\Development;D:\\\\Work\"\n\n[ai]\n# endpoint = \"http://localhost:11434/v1\"\n# model = \"qwen2.5-coder:14b\"\n",
            );
        }
        #[cfg(windows)]
        let _ = std::process::Command::new("cmd").args(["/C", "start", "", &path.to_string_lossy()]).spawn();
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("open").arg(&path).spawn();
        #[cfg(not(any(windows, target_os = "macos")))]
        let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
    });
}
