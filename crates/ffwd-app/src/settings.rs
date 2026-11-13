//! User settings in `settings.toml` (per-user config folder).
//!
//! The file is a small TOML subset: `[section]` headers, `key = value` with
//! quoted strings, booleans or integers, and `#` comments. Changes made in
//! the app rewrite only the line of the changed key, so the user's comments
//! and unknown keys survive. Reading happens on a worker thread at startup,
//! writing on a short-lived thread (performance rule 1).

use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    /// `section.key` → raw value text (strings without their quotes).
    values: BTreeMap<String, String>,
    /// Lines that did not parse, reported once in the status bar.
    pub errors: Vec<String>,
    /// Where changes are saved; `None` for in-memory settings (snapshots).
    file: Option<PathBuf>,
}

fn config_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"));
    #[cfg(not(any(windows, target_os = "macos")))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    base.map(|b| b.join("ffwd"))
}

/// The user's settings file. `FFWD_SETTINGS` points tests and agent runs
/// at a private file.
pub fn path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("FFWD_SETTINGS") {
        return Some(PathBuf::from(p));
    }
    config_dir().map(|d| d.join("settings.toml"))
}

/// ffwd's own state (open tabs, recent repositories), next to the settings
/// but separate, so the user's file stays theirs. `FFWD_STATE` overrides.
pub fn state_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("FFWD_STATE") {
        return Some(PathBuf::from(p));
    }
    config_dir().map(|d| d.join("state.toml"))
}

/// One writer at a time, in order: quick successive changes never race.
static WRITER: std::sync::Mutex<()> = std::sync::Mutex::new(());

impl Settings {
    pub fn load() -> Settings {
        Settings::load_from(path())
    }

    pub fn load_state() -> Settings {
        Settings::load_from(state_path())
    }

    fn load_from(file: Option<PathBuf>) -> Settings {
        let mut s = match file.as_ref().and_then(|p| std::fs::read_to_string(p).ok()) {
            Some(text) => Settings::parse(&text),
            None => Settings::default(),
        };
        s.file = file;
        s
    }

    pub fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        let mut section = String::new();
        for (n, raw) in text.lines().enumerate() {
            let line = strip_comment(raw).trim();
            if line.is_empty() {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = name.trim().to_string();
                continue;
            }
            match line.split_once('=') {
                Some((k, v)) if !k.trim().is_empty() => match parse_value(v.trim()) {
                    Some(v) => {
                        s.values.insert(full_key(&section, k.trim()), v);
                    }
                    None => s.errors.push(format!("settings.toml line {}: unreadable value", n + 1)),
                },
                _ => s.errors.push(format!("settings.toml line {}: expected key = value", n + 1)),
            }
        }
        s
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(|s| s.as_str())
    }

    /// Sets `key` (`section.name`) to a string. The file is read, rewritten
    /// and replaced on a worker thread, never on the UI thread.
    pub fn set_string(&mut self, key: &str, value: &str) {
        self.values.insert(key.to_string(), value.to_string());
        let Some(p) = self.file.clone() else { return };
        let (key, value) = (key.to_string(), format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"")));
        std::thread::spawn(move || {
            let _guard = WRITER.lock();
            let old = std::fs::read_to_string(&p).unwrap_or_default();
            let text = rewrite(&old, &key, &value);
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            // Write to a temporary file, then rename: a crash never leaves
            // a half-written file.
            let tmp = p.with_extension("toml.tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, &p);
            }
        });
    }

    /// A list stored as one string, entries separated by `|` (a character
    /// that cannot appear in Windows paths).
    pub fn get_list(&self, key: &str) -> Vec<String> {
        self.get(key).map(|s| s.split('|').filter(|x| !x.is_empty()).map(str::to_string).collect()).unwrap_or_default()
    }

    pub fn set_list(&mut self, key: &str, items: &[String]) {
        self.set_string(key, &items.join("|"));
    }
}
fn full_key(section: &str, key: &str) -> String {
    if section.is_empty() { key.to_string() } else { format!("{section}.{key}") }
}

fn strip_comment(line: &str) -> &str {
    let mut in_str = false;
    let mut prev = ' ';
    for (i, c) in line.char_indices() {
        match c {
            '"' if prev != '\\' => in_str = !in_str,
            '#' if !in_str => return &line[..i],
            _ => {}
        }
        prev = c;
    }
    line
}

fn parse_value(v: &str) -> Option<String> {
    if let Some(inner) = v.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        return Some(inner.replace("\\\"", "\"").replace("\\\\", "\\"));
    }
    if v == "true" || v == "false" || v.parse::<i64>().is_ok() {
        return Some(v.to_string());
    }
    None
}

/// Replaces the line of `key` in `text`, or adds it to its section (adding
/// the section at the end when it is missing). Everything else is kept.
fn rewrite(text: &str, key: &str, value: &str) -> String {
    let (section, name) = key.rsplit_once('.').unwrap_or(("", key));
    let entry = format!("{name} = {value}");
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut done = false;
    let mut first_header: Option<usize> = None;
    // Index just after the last non-blank line of the target section.
    let mut section_tail: Option<usize> = None;
    for raw in text.lines() {
        let line = strip_comment(raw).trim();
        if let Some(s) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            first_header.get_or_insert(out.len());
            current = s.trim().to_string();
        } else if !done
            && current == section
            && let Some((k, _)) = line.split_once('=')
            && k.trim() == name
        {
            out.push(entry.clone());
            done = true;
            continue;
        }
        out.push(raw.to_string());
        if current == section && !section.is_empty() && !raw.trim().is_empty() {
            section_tail = Some(out.len());
        }
    }
    if !done {
        if section.is_empty() {
            out.insert(first_header.unwrap_or(out.len()), entry);
        } else if let Some(i) = section_tail {
            out.insert(i, entry);
        } else {
            if out.last().is_some_and(|l| !l.trim().is_empty()) {
                out.push(String::new());
            }
            out.push(format!("[{section}]"));
            out.push(entry);
        }
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sections_values_and_comments() {
        let s = Settings::parse(
            "# mine\n[ui]\nlayout = \"beside\" # comment\nzoom = 2\nbad line\n[ai]\nendpoint = \"http://x/#a\"\n",
        );
        assert_eq!(s.get("ui.layout"), Some("beside"));
        assert_eq!(s.get("ui.zoom"), Some("2"));
        assert_eq!(s.get("ai.endpoint"), Some("http://x/#a"));
        assert_eq!(s.errors.len(), 1);
    }

    #[test]
    fn lists_round_trip() {
        let mut s = Settings::parse("");
        s.set_list("session.tabs", &[r"E:\a".into(), r"E:\b c".into()]);
        assert_eq!(s.get_list("session.tabs"), vec![r"E:\a".to_string(), r"E:\b c".to_string()]);
        assert!(Settings::parse("").get_list("x.y").is_empty());
    }

    #[test]
    fn rewrite_keeps_comments_and_other_keys() {
        let text = "# my settings\n[ui]\nlayout = \"below\" # old\nzoom = 2\n\n[ai]\nendpoint = \"x\"\n";
        let out = rewrite(text, "ui.layout", "\"beside\"");
        assert_eq!(out, "# my settings\n[ui]\nlayout = \"beside\"\nzoom = 2\n\n[ai]\nendpoint = \"x\"\n");
        // A key missing from an existing section goes into that section.
        let out = rewrite("[ui]\nzoom = 2\n\n[ai]\nendpoint = \"x\"\n", "ui.layout", "\"below\"");
        assert_eq!(Settings::parse(&out).get("ui.layout"), Some("below"));
        assert_eq!(Settings::parse(&out).get("ai.endpoint"), Some("x"));
        assert!(out.find("layout").unwrap() < out.find("[ai]").unwrap());
        // A missing section is added at the end.
        let out = rewrite("[ai]\nendpoint = \"x\"\n", "ui.layout", "\"below\"");
        assert!(out.ends_with("[ui]\nlayout = \"below\"\n"));
        assert_eq!(rewrite("", "ui.layout", "\"below\""), "[ui]\nlayout = \"below\"\n");
    }
}
