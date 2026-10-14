//! Minimal `~/.ssh/config` reader for importing hosts: Host, HostName, User, Port, IdentityFile.

use crate::paths::expand_tilde;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshConfigHost {
    pub alias: String,
    pub hostname: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_file: Option<String>,
}

#[derive(Default)]
struct Block {
    patterns: Vec<String>,
    /// `Match` blocks are not evaluated and never apply.
    is_match: bool,
    hostname: Option<String>,
    user: Option<String>,
    port: Option<u16>,
    identity_file: Option<String>,
}

fn split_line(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let (key, rest) = match line.find(|c: char| c.is_whitespace() || c == '=') {
        Some(i) => (&line[..i], &line[i..]),
        None => (line, ""),
    };
    let value = rest
        .trim_start_matches(|c: char| c.is_whitespace())
        .trim_start_matches('=')
        .trim();
    Some((key.to_ascii_lowercase(), value.to_owned()))
}

fn unquote(s: &str) -> String {
    s.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(s)
        .to_owned()
}

/// Glob match supporting `*` and `?`.
fn glob(pattern: &str, text: &str) -> bool {
    fn rec(p: &[char], t: &[char]) -> bool {
        match (p.first(), t.first()) {
            (None, None) => true,
            (Some('*'), _) => rec(&p[1..], t) || (!t.is_empty() && rec(p, &t[1..])),
            (Some('?'), Some(_)) => rec(&p[1..], &t[1..]),
            (Some(a), Some(b)) if a.eq_ignore_ascii_case(b) => rec(&p[1..], &t[1..]),
            _ => false,
        }
    }
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    rec(&p, &t)
}

fn block_matches(block: &Block, alias: &str) -> bool {
    if block.is_match {
        return false;
    }
    let mut matched = false;
    for pat in &block.patterns {
        if let Some(neg) = pat.strip_prefix('!') {
            if glob(neg, alias) {
                return false;
            }
        } else if glob(pat, alias) {
            matched = true;
        }
    }
    matched
}

fn expand_identity(value: &str, alias: &str, user: Option<&str>) -> String {
    let home = crate::paths::home_dir()
        .map(|h| h.display().to_string())
        .unwrap_or_default();
    let v = value
        .replace("%d", &home)
        .replace("%h", alias)
        .replace("%u", user.unwrap_or(""));
    expand_tilde(&v).display().to_string()
}

/// Parses the config text and returns one entry per concrete (non-pattern) host alias,
/// resolving values with OpenSSH's "first obtained value wins" rule.
pub fn parse(text: &str) -> Vec<SshConfigHost> {
    // Lines before the first Host apply to every host.
    let mut blocks = vec![Block {
        patterns: vec!["*".into()],
        ..Default::default()
    }];
    for raw in text.lines() {
        let Some((key, value)) = split_line(raw) else {
            continue;
        };
        match key.as_str() {
            "host" => blocks.push(Block {
                patterns: value.split_whitespace().map(unquote).collect(),
                ..Default::default()
            }),
            "match" => blocks.push(Block {
                is_match: true,
                ..Default::default()
            }),
            _ => {
                let Some(block) = blocks.last_mut() else {
                    continue;
                };
                let value = unquote(&value);
                match key.as_str() {
                    "hostname" if block.hostname.is_none() => block.hostname = Some(value),
                    "user" if block.user.is_none() => block.user = Some(value),
                    "port" if block.port.is_none() => block.port = value.parse().ok(),
                    "identityfile" if block.identity_file.is_none() => {
                        block.identity_file = Some(value)
                    }
                    _ => {}
                }
            }
        }
    }

    let mut aliases: Vec<String> = Vec::new();
    for block in &blocks[1..] {
        if block.is_match {
            continue;
        }
        for pat in &block.patterns {
            let concrete = !pat.contains(['*', '?', '!']);
            if concrete && !aliases.iter().any(|a| a == pat) {
                aliases.push(pat.clone());
            }
        }
    }

    aliases
        .into_iter()
        .map(|alias| {
            let mut entry = SshConfigHost {
                alias: alias.clone(),
                hostname: String::new(),
                user: None,
                port: None,
                identity_file: None,
            };
            let mut hostname = None;
            let mut identity = None;
            for block in blocks.iter().filter(|b| block_matches(b, &alias)) {
                if hostname.is_none() {
                    hostname = block.hostname.clone();
                }
                if entry.user.is_none() {
                    entry.user = block.user.clone();
                }
                if entry.port.is_none() {
                    entry.port = block.port;
                }
                if identity.is_none() {
                    identity = block.identity_file.clone();
                }
            }
            entry.hostname = hostname
                .map(|h| h.replace("%h", &alias))
                .unwrap_or_else(|| alias.clone());
            entry.identity_file =
                identity.map(|i| expand_identity(&i, &alias, entry.user.as_deref()));
            entry
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"
# global
User fallback

Host web prod-web
    HostName 10.0.0.5
    Port 2222
    IdentityFile ~/.ssh/id_web

Host db
  HostName=db.internal
  User = postgres

Host *.example.com
    User wildcard

Host bastion.example.com

Match host something
    User ignored

Host *
    Port 22
    IdentityFile ~/.ssh/id_default
"#;

    #[test]
    fn parses_concrete_hosts_with_first_value_wins() {
        let hosts = parse(CONFIG);
        let names: Vec<_> = hosts.iter().map(|h| h.alias.as_str()).collect();
        assert_eq!(names, ["web", "prod-web", "db", "bastion.example.com"]);

        let web = &hosts[0];
        assert_eq!(web.hostname, "10.0.0.5");
        assert_eq!(web.port, Some(2222));
        assert_eq!(web.user.as_deref(), Some("fallback"));
        assert!(web.identity_file.as_deref().unwrap().ends_with("id_web"));

        let db = &hosts[2];
        assert_eq!(db.hostname, "db.internal");
        assert_eq!(db.user.as_deref(), Some("fallback"));
        assert_eq!(db.port, Some(22));
        assert!(db.identity_file.as_deref().unwrap().ends_with("id_default"));

        let bastion = &hosts[3];
        assert_eq!(bastion.hostname, "bastion.example.com");
        assert_eq!(bastion.user.as_deref(), Some("fallback"));
    }

    #[test]
    fn negated_patterns_exclude() {
        let hosts = parse("Host a b\nHost * !b\n  User x\n");
        assert_eq!(hosts[0].user.as_deref(), Some("x"));
        assert_eq!(hosts[1].user, None);
    }

    #[test]
    fn glob_works() {
        assert!(glob("*.example.com", "a.example.com"));
        assert!(glob("h?st", "HOST"));
        assert!(!glob("*.example.com", "example.com"));
    }
}
