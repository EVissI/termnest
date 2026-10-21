//! Typing a saved sudo/su password, safely: only in answer to a password prompt that
//! shows up soon after the switch command, only once, and never after the user typed
//! something themselves. A password must never land in the shell as a command.

use std::time::{Duration, Instant};

use crate::secret::SecretString;
use crate::textutil::plain_text;

/// How long to wait for the prompt after the switch command.
const PROMPT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long to watch for "wrong password" after answering.
const VERDICT_TIMEOUT: Duration = Duration::from_secs(8);
const TAIL_BYTES: usize = 512;

/// A user switch typed into the shell: the command and, if saved, the password.
pub(crate) struct Switch {
    pub command: String,
    pub password: Option<SecretString>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// Keep watching.
    Wait,
    /// Send the password now (followed by Enter).
    Answer,
    /// The saved password was rejected; tell the user, do not retry.
    Rejected,
    /// Finished (success, timeout or cancelled): drop the feeder.
    Done,
}

pub(crate) struct AutoPassword {
    secret: SecretString,
    tail: Vec<u8>,
    deadline: Instant,
    answered: bool,
}

/// The last line of the terminal looks like a password prompt.
fn is_prompt(tail: &[u8]) -> bool {
    let text = plain_text(tail);
    let line = text
        .rsplit('\n')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_lowercase();
    line.ends_with(':') && (line.contains("password") || line.contains("пароль"))
}

fn is_rejection(tail: &[u8]) -> bool {
    let text = plain_text(tail).to_lowercase();
    [
        "try again",
        "authentication failure",
        "incorrect password",
        "ошибка аутентификации",
        "неверный пароль",
        "попробуйте ещё раз",
    ]
    .iter()
    .any(|m| text.contains(m))
}

impl AutoPassword {
    pub fn new(secret: SecretString, now: Instant) -> Self {
        Self {
            secret,
            tail: Vec::new(),
            deadline: now + PROMPT_TIMEOUT,
            answered: false,
        }
    }

    pub fn secret(&self) -> &SecretString {
        &self.secret
    }

    /// Server output arrived.
    pub fn output(&mut self, data: &[u8], now: Instant) -> Step {
        if now > self.deadline {
            return Step::Done;
        }
        self.tail.extend_from_slice(data);
        let cut = self.tail.len().saturating_sub(TAIL_BYTES);
        self.tail.drain(..cut);
        if self.answered {
            // A rejection, or the same prompt again, means the password did not work.
            if is_rejection(&self.tail) || is_prompt(&self.tail) {
                return Step::Rejected;
            }
            return Step::Wait;
        }
        if is_prompt(&self.tail) {
            self.answered = true;
            self.tail.clear();
            self.deadline = now + VERDICT_TIMEOUT;
            return Step::Answer;
        }
        Step::Wait
    }

    /// Called periodically: the feeder expires when nothing happened in time.
    pub fn expired(&self, now: Instant) -> bool {
        now > self.deadline
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feeder() -> (AutoPassword, Instant) {
        let now = Instant::now();
        (AutoPassword::new(SecretString::new("pw".into()), now), now)
    }

    #[test]
    fn answers_sudo_prompt_once() {
        let (mut f, now) = feeder();
        assert_eq!(f.output(b"deploy@box:~$ sudo -i\r\n", now), Step::Wait);
        assert_eq!(f.output(b"[sudo] password for deploy: ", now), Step::Answer);
        assert_eq!(f.output(b"\r\n", now), Step::Wait);
        assert_eq!(f.output(b"\x1b[01;31mroot@box\x1b[0m:~# ", now), Step::Wait);
    }

    #[test]
    fn su_and_russian_prompts() {
        assert!(is_prompt(b"su -\r\nPassword: "));
        assert!(is_prompt("Пароль: ".as_bytes()));
        assert!(!is_prompt(b"root@box:~# "));
        assert!(!is_prompt(b"Password: ok\r\n$ "));
    }

    #[test]
    fn wrong_password_is_reported_not_retried() {
        let (mut f, now) = feeder();
        assert_eq!(f.output(b"[sudo] password for deploy: ", now), Step::Answer);
        assert_eq!(f.output(b"\r\nSorry, try again.\r\n", now), Step::Rejected);
        let (mut f, now) = feeder();
        assert_eq!(f.output(b"Password: ", now), Step::Answer);
        assert_eq!(f.output(b"\r\nPassword: ", now), Step::Rejected);
    }

    #[test]
    fn late_prompt_is_ignored() {
        let (mut f, now) = feeder();
        // No prompt within the timeout (e.g. sudo did not ask): never answer later.
        let later = now + PROMPT_TIMEOUT + Duration::from_secs(1);
        assert!(f.expired(later));
        assert_eq!(f.output(b"Password: ", later), Step::Done);
    }
}
