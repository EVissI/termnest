//! Turning raw terminal output into plain text.

/// Removes ANSI escape sequences and applies carriage returns and backspaces the
/// way a terminal would, line by line. The result is plain text with `\n` endings.
pub fn plain_text(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let mut out = String::with_capacity(text.len());
    let mut line = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.next() {
                // CSI: parameters and intermediates up to a final byte 0x40..=0x7e.
                Some('[') => {
                    for n in chars.by_ref() {
                        if ('\x40'..='\x7e').contains(&n) {
                            break;
                        }
                    }
                }
                // OSC / DCS / APC / PM / SOS: up to BEL or ST (ESC \).
                Some(']') | Some('P') | Some('_') | Some('^') | Some('X') => {
                    while let Some(n) = chars.next() {
                        if n == '\x07' {
                            break;
                        }
                        if n == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                // Charset designation and similar two-character sequences.
                Some('(') | Some(')') | Some('*') | Some('+') => {
                    chars.next();
                }
                _ => {}
            },
            '\n' => {
                out.push_str(line.trim_end());
                out.push('\n');
                line.clear();
            }
            '\r' => {
                if chars.peek() != Some(&'\n') {
                    line.clear(); // the next text overwrites the line
                }
            }
            '\x08' => {
                line.pop();
            }
            c if c.is_control() && c != '\t' => {}
            c => line.push(c),
        }
    }
    out.push_str(line.trim_end());
    out
}

/// The last `lines` lines of a text.
pub fn tail_lines(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    let start = all.len().saturating_sub(lines);
    all[start..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_escapes_and_applies_cr() {
        let raw = b"\x1b]0;user@host: ~\x07\x1b[01;32muser@host\x1b[00m:~$ ls\r\nfile1  file2\r\n10%\r20%\r100% done\r\nab\x08c\n";
        assert_eq!(
            plain_text(raw),
            "user@host:~$ ls\nfile1  file2\n100% done\nac\n"
        );
    }

    #[test]
    fn tail() {
        assert_eq!(tail_lines("a\nb\nc\nd", 2), "c\nd");
        assert_eq!(tail_lines("a", 5), "a");
    }
}
