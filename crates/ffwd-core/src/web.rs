//! Web links for a repository's hosting site (FF-33, FF-40): the page of
//! a commit, from the remote URL. GitHub, GitLab, Bitbucket and Azure
//! DevOps have their own forms; other hosts (Gitea, Forgejo, Codeberg)
//! use GitHub's.

/// `(host, path)` of a remote URL in https, ssh or scp form, without
/// `.git`, user or port.
fn host_path(url: &str) -> Option<(String, String)> {
    let url = url.trim();
    let (host, path) = if let Some((scheme, rest)) = url.split_once("://") {
        if !matches!(scheme, "https" | "http" | "ssh" | "git") {
            return None;
        }
        rest.split_once('/')?
    } else {
        let (h, p) = url.split_once(':')?;
        if h.len() < 2 || h.contains('/') {
            return None;
        }
        (h, p)
    };
    let host = host.rsplit_once('@').map_or(host, |x| x.1);
    let host = host.split_once(':').map_or(host, |x| x.0).to_ascii_lowercase();
    let path = path.trim_matches('/').trim_end_matches(".git").to_string();
    if host.is_empty() || path.is_empty() {
        return None;
    }
    Some((host, path))
}

/// The host name shown in menus ("github.com").
pub fn host(url: &str) -> Option<String> {
    host_path(url).map(|(h, _)| h.trim_start_matches("ssh.").to_string())
}

/// The web page of commit `sha` for a repository at `remote_url`.
pub fn commit_url(remote_url: &str, sha: &str) -> Option<String> {
    let (host, path) = host_path(remote_url)?;
    if !sha.chars().all(|c| c.is_ascii_hexdigit()) || sha.is_empty() {
        return None;
    }
    Some(if host == "ssh.dev.azure.com" || host == "dev.azure.com" || host.ends_with(".visualstudio.com") {
        // ssh: v3/org/project/repo; https: org/project/_git/repo
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty() && *p != "v3").collect();
        let (org, project, repo) = match parts.as_slice() {
            [org, project, "_git", repo] => (*org, *project, *repo),
            [org, project, repo] => (*org, *project, *repo),
            _ => return None,
        };
        format!("https://dev.azure.com/{org}/{project}/_git/{repo}/commit/{sha}")
    } else if host == "bitbucket.org" {
        format!("https://bitbucket.org/{path}/commits/{sha}")
    } else if host.contains("gitlab") {
        format!("https://{host}/{path}/-/commit/{sha}")
    } else {
        format!("https://{host}/{path}/commit/{sha}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_pages() {
        let s = "0123abcd";
        assert_eq!(
            commit_url("https://github.com/rennerdo30/ffwd.git", s).unwrap(),
            "https://github.com/rennerdo30/ffwd/commit/0123abcd"
        );
        assert_eq!(
            commit_url("git@github.com:rennerdo30/ffwd.git", s).unwrap(),
            "https://github.com/rennerdo30/ffwd/commit/0123abcd"
        );
        assert_eq!(
            commit_url("ssh://git@gitlab.example.com:2222/group/sub/app.git", s).unwrap(),
            "https://gitlab.example.com/group/sub/app/-/commit/0123abcd"
        );
        assert_eq!(
            commit_url("git@bitbucket.org:team/app.git", s).unwrap(),
            "https://bitbucket.org/team/app/commits/0123abcd"
        );
        assert_eq!(
            commit_url("git@ssh.dev.azure.com:v3/org/proj/repo", s).unwrap(),
            "https://dev.azure.com/org/proj/_git/repo/commit/0123abcd"
        );
        assert_eq!(
            commit_url("https://org@dev.azure.com/org/proj/_git/repo", s).unwrap(),
            "https://dev.azure.com/org/proj/_git/repo/commit/0123abcd"
        );
        assert_eq!(commit_url("https://codeberg.org/me/x", s).unwrap(), "https://codeberg.org/me/x/commit/0123abcd");
        assert_eq!(commit_url("C:/repos/x", s), None);
        assert_eq!(commit_url("file:///repos/x", s), None);
        assert_eq!(commit_url("https://github.com/a/b", "x;rm"), None);
        assert_eq!(host("git@ssh.dev.azure.com:v3/o/p/r").as_deref(), Some("dev.azure.com"));
    }
}
