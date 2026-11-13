//! The repository's files (FF-51, FF-43): the file list as a tree, a
//! file's content, its history and its blame. Bounded everywhere: the
//! target is 100k files, and a huge or binary file never loads whole.

use crate::git::{Git, GitError};
use crate::log::Oid;

/// A folder tree built from paths. Node 0 is the root.
#[derive(Clone, Debug, Default)]
pub struct Tree {
    pub nodes: Vec<Node>,
    /// More than `MAX_FILES` files: the rest is not listed.
    pub truncated: bool,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub name: Box<str>,
    pub parent: u32,
    /// Folders first, then files, each sorted by name (case-insensitive).
    pub children: Vec<u32>,
    pub dir: bool,
    /// Working tree only: `M`, `A`, `D`, `?` (untracked) or `U`; on a
    /// folder, set when anything inside changed.
    pub mark: u8,
}

pub const MAX_FILES: usize = 300_000;

impl Tree {
    /// Builds the tree from `/`-separated paths with their marks.
    pub fn build(paths: impl IntoIterator<Item = (String, u8)>) -> Tree {
        let mut t = Tree {
            nodes: vec![Node { name: "".into(), parent: 0, children: Vec::new(), dir: true, mark: 0 }],
            truncated: false,
        };
        let mut index: std::collections::HashMap<(u32, Box<str>), u32> = std::collections::HashMap::new();
        for (n, (path, mark)) in paths.into_iter().enumerate() {
            if n >= MAX_FILES {
                t.truncated = true;
                break;
            }
            let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
            let mut cur = 0u32;
            for (i, part) in parts.iter().enumerate() {
                let last = i + 1 == parts.len();
                let key = (cur, Box::<str>::from(*part));
                let id = match index.get(&key) {
                    Some(&id) => id,
                    None => {
                        let id = t.nodes.len() as u32;
                        t.nodes.push(Node {
                            name: (*part).into(),
                            parent: cur,
                            children: Vec::new(),
                            dir: !last,
                            mark: 0,
                        });
                        t.nodes[cur as usize].children.push(id);
                        index.insert(key, id);
                        id
                    }
                };
                if last {
                    t.nodes[id as usize].mark = mark;
                }
                cur = id;
            }
            // Folders above a change carry a mark too.
            if mark != 0 {
                let mut p = t.nodes[cur as usize].parent;
                while p != 0 {
                    if t.nodes[p as usize].mark == 0 {
                        t.nodes[p as usize].mark = b'M';
                    }
                    p = t.nodes[p as usize].parent;
                }
            }
        }
        // Sort every folder: folders first, then by name.
        let names: Vec<(bool, String)> = t.nodes.iter().map(|n| (n.dir, n.name.to_lowercase())).collect();
        for n in &mut t.nodes {
            n.children.sort_by(|&a, &b| {
                let (x, y) = (&names[a as usize], &names[b as usize]);
                y.0.cmp(&x.0).then_with(|| x.1.cmp(&y.1))
            });
        }
        t
    }

    /// The full path of a node.
    pub fn path(&self, mut id: u32) -> String {
        let mut parts = Vec::new();
        while id != 0 {
            let n = &self.nodes[id as usize];
            parts.push(n.name.as_ref());
            id = n.parent;
        }
        parts.reverse();
        parts.join("/")
    }

    /// The node at `path`, walking only the folders on the way.
    pub fn find(&self, path: &str) -> Option<u32> {
        let mut id = 0u32;
        for part in path.split('/').filter(|p| !p.is_empty()) {
            id = *self.nodes[id as usize].children.iter().find(|&&c| &*self.nodes[c as usize].name == part)?;
        }
        (id != 0).then_some(id)
    }
}

fn split_z(b: &[u8]) -> impl Iterator<Item = String> + '_ {
    b.split(|&c| c == 0).filter(|s| !s.is_empty()).map(|s| String::from_utf8_lossy(s).into_owned())
}

/// The working tree's files: tracked ones plus untracked, not ignored
/// ones, with change marks from `status` (path → letter).
pub fn list_worktree(git: &Git, marks: &std::collections::HashMap<String, u8>) -> Result<Tree, GitError> {
    let tracked = git.output_limited(&["ls-files", "-z", "--cached"], 128 << 20)?;
    let others = git.output_limited(&["ls-files", "-z", "--others", "--exclude-standard"], 64 << 20)?;
    let mut paths: Vec<(String, u8)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for p in split_z(&tracked) {
        if seen.insert(p.clone()) {
            let m = marks.get(&p).copied().unwrap_or(0);
            paths.push((p, m));
        }
    }
    for p in split_z(&others) {
        if seen.insert(p.clone()) {
            paths.push((p, b'?'));
        }
    }
    Ok(Tree::build(paths))
}

/// The files of a commit.
pub fn list_at(git: &Git, at: Oid) -> Result<Tree, GitError> {
    let out = git.output_limited(&["ls-tree", "-r", "-z", "--name-only", &at.to_string()], 128 << 20)?;
    Ok(Tree::build(split_z(&out).map(|p| (p, 0))))
}

#[derive(Clone, Debug, Default)]
pub struct Content {
    pub lines: Vec<Box<str>>,
    pub binary: bool,
    /// Longer than `MAX_CONTENT`: only the start is shown.
    pub truncated: bool,
    pub bytes: u64,
}

pub const MAX_CONTENT: usize = 4 << 20;

fn to_content(b: &[u8], total: u64) -> Content {
    let binary = b.iter().take(8000).any(|&c| c == 0);
    if binary {
        return Content { binary: true, bytes: total, ..Default::default() };
    }
    let s = String::from_utf8_lossy(b);
    // A final newline ends the last line; it does not start another.
    let body = s.strip_suffix('\n').unwrap_or(&s);
    if body.is_empty() {
        return Content { lines: Vec::new(), binary: false, truncated: false, bytes: total };
    }
    let lines = body.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l).into()).collect();
    Content { lines, binary: false, truncated: total as usize > b.len(), bytes: total }
}

/// A file's content: from the working tree (`at = None`) or a commit.
pub fn read(git: &Git, at: Option<Oid>, path: &str) -> Result<Content, String> {
    match at {
        None => {
            use std::io::Read;
            let p = git.root.join(path);
            let total = std::fs::metadata(&p).map_err(|e| format!("{path}: {e}"))?.len();
            let mut f = std::fs::File::open(&p).map_err(|e| format!("{path}: {e}"))?;
            let mut b = Vec::new();
            f.by_ref().take(MAX_CONTENT as u64).read_to_end(&mut b).map_err(|e| format!("{path}: {e}"))?;
            Ok(to_content(&b, total))
        }
        Some(id) => {
            let spec = format!("{id}:{path}");
            let b = git.output_limited(&["cat-file", "blob", &spec], MAX_CONTENT).map_err(|e| e.to_string())?;
            let total = git
                .output_limited(&["cat-file", "-s", &spec], 64)
                .ok()
                .and_then(|s| String::from_utf8_lossy(&s).trim().parse().ok())
                .unwrap_or(b.len() as u64);
            Ok(to_content(&b, total))
        }
    }
}

/// The commits that changed `path`, newest first, following renames.
pub fn history(git: &Git, path: &str) -> Result<Vec<Oid>, GitError> {
    let out = git.output_limited(&["log", "--follow", "--format=%H", "-n", "5000", "--", path], 4 << 20)?;
    Ok(out.split(|&c| c == b'\n').filter_map(|l| Oid::from_hex(l.trim_ascii())).collect())
}

#[derive(Clone, Debug)]
pub struct BlameCommit {
    pub id: Oid,
    pub author: Box<str>,
    pub time: i64,
    pub summary: Box<str>,
}

/// Per line, an index into `commits`.
#[derive(Clone, Debug, Default)]
pub struct Blame {
    pub commits: Vec<BlameCommit>,
    pub lines: Vec<u32>,
}

/// `git blame --porcelain` of a file at a commit (`None`: the working tree).
pub fn blame(git: &Git, at: Option<Oid>, path: &str) -> Result<Blame, GitError> {
    let rev = at.map(|a| a.to_string());
    let mut args = vec!["blame", "--porcelain"];
    if let Some(r) = &rev {
        args.push(r);
    }
    args.extend(["--", path]);
    let out = git.output_limited(&args, 256 << 20)?;
    Ok(parse_blame(&out))
}

pub fn parse_blame(out: &[u8]) -> Blame {
    let mut b = Blame::default();
    let mut by_id: std::collections::HashMap<Oid, u32> = std::collections::HashMap::new();
    let mut cur: Option<u32> = None;
    for raw in out.split(|&c| c == b'\n') {
        if raw.starts_with(b"\t") {
            if let Some(c) = cur {
                b.lines.push(c);
            }
            continue;
        }
        let line = String::from_utf8_lossy(raw);
        let mut parts = line.splitn(2, ' ');
        let first = parts.next().unwrap_or("");
        if let Some(id) = Oid::from_hex(first.as_bytes()) {
            let idx = *by_id.entry(id).or_insert_with(|| {
                b.commits.push(BlameCommit { id, author: "".into(), time: 0, summary: "".into() });
                (b.commits.len() - 1) as u32
            });
            cur = Some(idx);
            continue;
        }
        let rest = parts.next().unwrap_or("");
        if let Some(c) = cur.map(|c| &mut b.commits[c as usize]) {
            match first {
                "author" => c.author = rest.into(),
                "author-time" => c.time = rest.trim().parse().unwrap_or(0),
                "summary" => c.summary = rest.into(),
                _ => {}
            }
        }
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_sorts_folders_first_and_marks_parents() {
        let t = Tree::build([
            ("src/main.rs".to_string(), 0),
            ("README.md".to_string(), 0),
            ("src/ui/panel.rs".to_string(), b'M'),
            ("Cargo.toml".to_string(), 0),
        ]);
        let root: Vec<&str> = t.nodes[0].children.iter().map(|&c| &*t.nodes[c as usize].name).collect();
        assert_eq!(root, vec!["src", "Cargo.toml", "README.md"]);
        let src = t.nodes[0].children[0];
        assert_eq!(t.nodes[src as usize].mark, b'M', "a folder above a change is marked");
        let ui = t.nodes[src as usize].children[0];
        assert_eq!(t.path(t.nodes[ui as usize].children[0]), "src/ui/panel.rs");
    }

    #[test]
    fn worktree_content_history_and_blame() {
        let dir = std::env::temp_dir().join(format!("ffwd-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        let g = Git { root: dir.clone() };
        for a in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "Ada"],
            &["config", "user.email", "ada@example.invalid"],
            &["config", "commit.gpgsign", "false"],
        ] {
            g.output(a).unwrap();
        }
        std::fs::write(dir.join("src/a.rs"), "one\ntwo\n").unwrap();
        g.output(&["add", "."]).unwrap();
        g.output(&["commit", "-q", "-m", "first"]).unwrap();
        g.output(&["config", "user.name", "Bob"]).unwrap();
        std::fs::write(dir.join("src/a.rs"), "one\nTWO\nthree\n").unwrap();
        g.output(&["commit", "-qam", "second"]).unwrap();
        std::fs::write(dir.join("new.txt"), "x\n").unwrap();
        std::fs::write(dir.join("bin.dat"), [0u8, 1, 2, 0]).unwrap();
        let t = list_worktree(&g, &std::collections::HashMap::new()).unwrap();
        let names: Vec<String> =
            (1..t.nodes.len() as u32).filter(|&i| !t.nodes[i as usize].dir).map(|i| t.path(i)).collect();
        assert!(names.contains(&"src/a.rs".to_string()) && names.contains(&"new.txt".to_string()));
        let new = (1..t.nodes.len() as u32).find(|&i| t.path(i) == "new.txt").unwrap();
        assert_eq!(t.nodes[new as usize].mark, b'?');
        let c = read(&g, None, "src/a.rs").unwrap();
        assert_eq!(c.lines.iter().map(|l| &**l).collect::<Vec<_>>(), vec!["one", "TWO", "three"]);
        assert!(read(&g, None, "bin.dat").unwrap().binary);
        let head = Oid::from_hex(g.output(&["rev-parse", "HEAD~1"]).unwrap().trim_ascii()).unwrap();
        assert_eq!(read(&g, Some(head), "src/a.rs").unwrap().lines[1].as_ref(), "two");
        assert_eq!(list_at(&g, head).unwrap().nodes.iter().filter(|n| !n.dir).count(), 1);
        assert_eq!(history(&g, "src/a.rs").unwrap().len(), 2);
        let b = blame(&g, None, "src/a.rs").unwrap();
        assert_eq!(b.lines.len(), 3);
        let who: Vec<&str> = b.lines.iter().map(|&i| &*b.commits[i as usize].author).collect();
        assert_eq!(who, vec!["Ada", "Bob", "Bob"]);
        assert_eq!(&*b.commits[b.lines[1] as usize].summary, "second");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
