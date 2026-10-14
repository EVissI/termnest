use std::fmt;
use std::str::FromStr;

use serde::Serialize;

use crate::error::{CoreError, ErrorKind};

pub const DEFAULT_PORT: u16 = 22;

/// Connection target parsed from `user@host[:port]`.
///
/// IPv6 addresses must be written in brackets: `user@[::1]:22`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Target {
    pub user: String,
    pub host: String,
    pub port: u16,
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.host.contains(':') {
            write!(f, "{}@[{}]:{}", self.user, self.host, self.port)
        } else {
            write!(f, "{}@{}:{}", self.user, self.host, self.port)
        }
    }
}

fn invalid(message: &str) -> CoreError {
    CoreError::new(ErrorKind::InvalidTarget, message)
}

impl FromStr for Target {
    type Err = CoreError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let s = input.trim();
        if s.is_empty() {
            return Err(invalid("Укажите адрес в виде user@host:port"));
        }
        if s.chars().any(char::is_whitespace) {
            return Err(invalid("Адрес не должен содержать пробелов"));
        }

        let (user, rest) = s
            .rsplit_once('@')
            .ok_or_else(|| invalid("Не указан пользователь: нужно user@host:port"))?;
        if user.is_empty() {
            return Err(invalid("Не указан пользователь"));
        }

        let (host, port) = if let Some(after_bracket) = rest.strip_prefix('[') {
            let (host, tail) = after_bracket
                .split_once(']')
                .ok_or_else(|| invalid("Не закрыта скобка в IPv6-адресе"))?;
            let port = match tail {
                "" => None,
                t => Some(
                    t.strip_prefix(':')
                        .ok_or_else(|| invalid("После IPv6-адреса ожидается :port"))?,
                ),
            };
            (host, port)
        } else {
            match rest.split_once(':') {
                Some((h, p)) => (h, Some(p)),
                None => (rest, None),
            }
        };

        if host.is_empty() {
            return Err(invalid("Не указан хост"));
        }

        let port = match port {
            None => DEFAULT_PORT,
            Some(p) => match p.parse::<u16>() {
                Ok(n) if n > 0 => n,
                _ => return Err(invalid("Порт должен быть числом от 1 до 65535")),
            },
        };

        Ok(Target {
            user: user.to_owned(),
            host: host.to_owned(),
            port,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Target {
        s.parse().unwrap()
    }

    #[test]
    fn parses_full_form() {
        assert_eq!(
            t("root@example.com:2222"),
            Target {
                user: "root".into(),
                host: "example.com".into(),
                port: 2222
            }
        );
    }

    #[test]
    fn default_port() {
        assert_eq!(t("me@10.0.0.1").port, 22);
    }

    #[test]
    fn trims_whitespace_around() {
        assert_eq!(t("  me@host:22 \n").host, "host");
    }

    #[test]
    fn ipv6_in_brackets() {
        let target = t("me@[::1]:2200");
        assert_eq!(target.host, "::1");
        assert_eq!(target.port, 2200);
        assert_eq!(t("me@[fe80::1]").port, 22);
        assert_eq!(target.to_string(), "me@[::1]:2200");
    }

    #[test]
    fn user_may_contain_at() {
        let target = t("a@b@host");
        assert_eq!(target.user, "a@b");
        assert_eq!(target.host, "host");
    }

    #[test]
    fn rejects_invalid() {
        for s in [
            "",
            "host",
            "@host",
            "me@",
            "me@:22",
            "me@host:",
            "me@host:0",
            "me@host:70000",
            "me@host:abc",
            "me@[::1",
            "me@[::1]22",
            "me @host",
        ] {
            let err = s.parse::<Target>().unwrap_err();
            assert_eq!(err.kind, ErrorKind::InvalidTarget, "input: {s:?}");
        }
    }
}
