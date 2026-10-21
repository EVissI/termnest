//! Lifecycle of one session: connect, pump data, reconnect with backoff after a drop.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use russh::client::Msg;
use russh::{Channel, ChannelMsg};
use tokio::sync::mpsc;
use zeroize::Zeroizing;

use super::autopass::{AutoPassword, Step, Switch};
use super::{Command, ConnectSpec, SessionEntry, SessionState};
use crate::error::{CoreError, CoreResult, ErrorKind};
use crate::ssh::{self, Auth, SshContext};

const TERM: &str = "xterm-256color";
const MAX_BACKOFF_SECS: u64 = 30;

/// Delay before reconnect attempt `n` (1-based): 1, 2, 4, ... capped at 30 seconds.
pub(crate) fn backoff(attempt: u32) -> Duration {
    let exp = attempt.saturating_sub(1).min(16);
    Duration::from_secs((1u64 << exp).min(MAX_BACKOFF_SECS))
}

enum End {
    /// The user closed the session.
    ByUser,
    /// The remote shell ended (e.g. `exit`); do not reconnect.
    Exited(Option<u32>),
    /// The connection dropped; reconnect.
    Lost,
}

enum Next {
    Connect,
    Stop,
}

struct Runner {
    entry: Arc<SessionEntry>,
    ctx: Arc<SshContext>,
    rx: mpsc::UnboundedReceiver<Command>,
    size: (u32, u32),
}

pub(super) async fn run(
    entry: Arc<SessionEntry>,
    ctx: Arc<SshContext>,
    rx: mpsc::UnboundedReceiver<Command>,
    size: (u32, u32),
) {
    let mut r = Runner {
        entry,
        ctx,
        rx,
        size,
    };
    r.main().await;
    r.entry.set_handle(None);
    r.entry.drop_sftp().await;
}

impl Runner {
    async fn main(&mut self) {
        let spec = self.entry.spec.clone();
        let mut reconnecting = false;
        let mut attempt: u32 = 0;
        loop {
            if reconnecting {
                self.entry.publish(|s| {
                    s.state = SessionState::Reconnecting;
                    s.retry_in_ms = None;
                    s.attempt = attempt;
                });
            } else {
                self.entry
                    .notice(&format!("Подключение к {}...", spec.target()), 90);
                self.entry.publish(|s| {
                    s.state = SessionState::Connecting;
                    s.error = None;
                    s.retry_in_ms = None;
                    s.attempt = 0;
                    s.exit_code = None;
                });
            }

            let Some(result) = self.connect(&spec).await else {
                return; // closed while connecting
            };

            let err = match result {
                Ok((handle, channel)) => {
                    let handle = Arc::new(handle);
                    self.entry.set_handle(Some(handle.clone()));
                    if reconnecting {
                        self.entry.notice("Переподключено", 32);
                    }
                    self.entry.publish(|s| {
                        s.state = SessionState::Connected;
                        s.error = None;
                        s.retry_in_ms = None;
                        s.attempt = 0;
                    });
                    reconnecting = false;
                    attempt = 0;

                    let end = self.pump(channel, spec.startup_switch()).await;
                    self.entry.set_handle(None);
                    self.entry.drop_sftp().await;
                    ssh::disconnect(&handle).await;

                    match end {
                        End::ByUser => {
                            self.entry.publish(|s| {
                                s.state = SessionState::Disconnected;
                                s.error = None;
                            });
                            return;
                        }
                        End::Exited(code) => {
                            let text = match code {
                                Some(c) => format!("Сеанс завершён (код {c})"),
                                None => "Сеанс завершён".to_owned(),
                            };
                            self.entry.notice(&text, 90);
                            self.entry.publish(|s| {
                                s.state = SessionState::Disconnected;
                                s.error = None;
                                s.exit_code = code;
                            });
                            match self.idle().await {
                                Next::Connect => continue,
                                Next::Stop => return,
                            }
                        }
                        End::Lost => {
                            self.entry.notice("Соединение потеряно", 33);
                            reconnecting = true;
                            CoreError::new(ErrorKind::Unreachable, "Соединение потеряно")
                        }
                    }
                }
                Err(e) => e,
            };

            if reconnecting && err.is_transient() {
                attempt += 1;
                match self.wait_backoff(attempt, &err).await {
                    Next::Connect => continue,
                    Next::Stop => return,
                }
            }

            // Permanent error (or the first connect failed): show it and wait for Retry.
            self.entry.notice(&err.message, 31);
            self.entry.publish(|s| {
                s.state = SessionState::Error;
                s.error = Some(err.clone());
                s.retry_in_ms = None;
            });
            reconnecting = false;
            attempt = 0;
            match self.idle().await {
                Next::Connect => continue,
                Next::Stop => return,
            }
        }
    }

    /// Connects while still reacting to Close and Resize. `None` means closed.
    async fn connect(
        &mut self,
        spec: &ConnectSpec,
    ) -> Option<CoreResult<(russh::client::Handle<ssh::Client>, Channel<Msg>)>> {
        let ctx = self.ctx.clone();
        let target = spec.target();
        let key_path = PathBuf::from(&spec.key_path);
        let size = self.size;
        let work = async move {
            if spec_key_missing(&key_path) {
                return Err(CoreError::new(
                    ErrorKind::KeyNotFound,
                    "Не указан ключ для подключения",
                ));
            }
            let handle = ssh::connect(&target, Auth::Key(&key_path), &ctx).await?;
            let channel = open_shell(&handle, size).await?;
            Ok((handle, channel))
        };
        tokio::pin!(work);
        loop {
            tokio::select! {
                result = &mut work => return Some(result),
                cmd = self.rx.recv() => match cmd {
                    None | Some(Command::Close) => return None,
                    Some(Command::Resize { cols, rows }) => self.size = (cols, rows),
                    Some(_) => {}
                },
            }
        }
    }

    async fn pump(&mut self, channel: Channel<Msg>, startup: Option<Switch>) -> End {
        let (mut reader, writer) = channel.split();
        // The size may have changed while the PTY was being requested.
        let _ = writer.window_change(self.size.0, self.size.1, 0, 0).await;
        let mut auto: Option<AutoPassword> = None;
        // The terminal buffers it until the shell reads input, like typing ahead.
        if let Some(sw) = startup {
            let _ = writer.data(format!("{}\r", sw.command).as_bytes()).await;
            auto = sw.password.map(|p| AutoPassword::new(p, Instant::now()));
        }
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        let mut exit_code = None;
        let mut exited = false;
        loop {
            tokio::select! {
                msg = reader.wait() => match msg {
                    Some(ChannelMsg::Data { data }) => {
                        self.entry.push_output(&data);
                        if let Some(f) = auto.as_mut() {
                            match f.output(&data, Instant::now()) {
                                Step::Wait => {}
                                Step::Answer => {
                                    let mut line = Zeroizing::new(f.secret().expose().as_bytes().to_vec());
                                    line.push(b'\r');
                                    if writer.data(&line[..]).await.is_err() {
                                        return End::Lost;
                                    }
                                }
                                Step::Rejected => {
                                    auto = None;
                                    self.entry.notice("termnest: сохранённый пароль не подошёл, введите его вручную и обновите в настройках сервера", 33);
                                }
                                Step::Done => auto = None,
                            }
                        }
                    }
                    Some(ChannelMsg::ExtendedData { data, .. }) => self.entry.push_output(&data),
                    Some(ChannelMsg::ExitStatus { exit_status }) => {
                        exit_code = Some(exit_status);
                        exited = true;
                    }
                    Some(ChannelMsg::ExitSignal { .. }) => exited = true,
                    Some(ChannelMsg::Close) | None => {
                        return if exited { End::Exited(exit_code) } else { End::Lost };
                    }
                    Some(_) => {}
                },
                _ = tick.tick(), if auto.is_some() => {
                    if auto.as_ref().is_some_and(|f| f.expired(Instant::now())) {
                        auto = None;
                    }
                }
                cmd = self.rx.recv() => match cmd {
                    Some(Command::Data(bytes)) => {
                        // The user answers or types something else: stop watching.
                        auto = None;
                        if writer.data(&bytes[..]).await.is_err() {
                            return End::Lost;
                        }
                    }
                    Some(Command::Switch(sw)) => {
                        if writer.data(format!("{}\r", sw.command).as_bytes()).await.is_err() {
                            return End::Lost;
                        }
                        auto = sw.password.map(|p| AutoPassword::new(p, Instant::now()));
                    }
                    Some(Command::Resize { cols, rows }) => {
                        self.size = (cols, rows);
                        let _ = writer.window_change(cols, rows, 0, 0).await;
                    }
                    Some(Command::Close) | None => {
                        let _ = writer.close().await;
                        return End::ByUser;
                    }
                    Some(_) => {}
                },
            }
        }
    }

    /// Waits for the reconnect delay; the user may skip it or cancel.
    async fn wait_backoff(&mut self, attempt: u32, err: &CoreError) -> Next {
        let delay = backoff(attempt);
        self.entry.publish(|s| {
            s.state = SessionState::Reconnecting;
            s.error = Some(err.clone());
            s.attempt = attempt;
            s.retry_in_ms = Some(delay.as_millis() as u64);
        });
        let sleep = tokio::time::sleep(delay);
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                _ = &mut sleep => return Next::Connect,
                cmd = self.rx.recv() => match cmd {
                    None | Some(Command::Close) => return Next::Stop,
                    Some(Command::ReconnectNow) | Some(Command::Retry) => return Next::Connect,
                    Some(Command::CancelReconnect) => {
                        self.entry.notice("Переподключение отменено", 90);
                        self.entry.publish(|s| {
                            s.state = SessionState::Disconnected;
                            s.retry_in_ms = None;
                        });
                        return self.idle().await;
                    }
                    Some(Command::Resize { cols, rows }) => self.size = (cols, rows),
                    Some(_) => {}
                },
            }
        }
    }

    /// Disconnected or failed: wait until the user asks to connect again or closes.
    async fn idle(&mut self) -> Next {
        loop {
            match self.rx.recv().await {
                None | Some(Command::Close) => return Next::Stop,
                Some(Command::Retry) | Some(Command::ReconnectNow) => return Next::Connect,
                Some(Command::Resize { cols, rows }) => self.size = (cols, rows),
                Some(_) => {}
            }
        }
    }
}

fn spec_key_missing(path: &std::path::Path) -> bool {
    path.as_os_str().is_empty()
}

async fn open_shell(
    handle: &russh::client::Handle<ssh::Client>,
    size: (u32, u32),
) -> CoreResult<Channel<Msg>> {
    let channel = handle
        .channel_open_session()
        .await
        .map_err(ssh::map_ssh_error)?;
    channel
        .request_pty(false, TERM, size.0, size.1, 0, 0, &[])
        .await
        .map_err(ssh::map_ssh_error)?;
    let _ = channel.set_env(false, "COLORTERM", "truecolor").await;
    channel
        .request_shell(false)
        .await
        .map_err(ssh::map_ssh_error)?;
    Ok(channel)
}

#[cfg(test)]
mod tests {
    use super::backoff;
    use std::time::Duration;

    #[test]
    fn backoff_doubles_up_to_30s() {
        let secs: Vec<u64> = (1..=8).map(|n| backoff(n).as_secs()).collect();
        assert_eq!(secs, [1, 2, 4, 8, 16, 30, 30, 30]);
        assert_eq!(backoff(1000), Duration::from_secs(30));
    }
}
