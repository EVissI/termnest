use std::io;
use std::path::Path;
use std::sync::Arc;

use russh::client::{self, Handle, Msg};
use russh::keys::{self, PrivateKey, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::{Channel, ChannelMsg, Disconnect};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::timeout;

use super::{Command, ConnectOptions, SessionEntry, SessionState};
use crate::error::{CoreError, CoreResult, ErrorKind};

const TERM: &str = "xterm-256color";

struct Client;

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        // Host key verification against known_hosts is added in stage 2.
        Ok(true)
    }
}

enum End {
    ByUser,
    ByRemote,
}

pub(super) async fn run(
    entry: Arc<SessionEntry>,
    options: ConnectOptions,
    mut commands: mpsc::UnboundedReceiver<Command>,
) {
    entry.set_state(SessionState::Connecting, None);
    let (handle, channel) = match establish(&options).await {
        Ok(conn) => conn,
        Err(err) => {
            log::warn!("session {}: connect failed: {}", entry.id, err);
            entry.set_state(SessionState::Error, Some(&err));
            return;
        }
    };
    entry.set_state(SessionState::Connected, None);

    let end = pump(&entry, channel, &mut commands).await;
    let _ = handle.disconnect(Disconnect::ByApplication, "", "en").await;

    match end {
        End::ByUser => entry.set_state(SessionState::Disconnected, None),
        End::ByRemote => entry.set_state(
            SessionState::Disconnected,
            Some(&CoreError::new(
                ErrorKind::Unreachable,
                "Соединение закрыто",
            )),
        ),
    }
}

async fn pump(
    entry: &SessionEntry,
    channel: Channel<Msg>,
    commands: &mut mpsc::UnboundedReceiver<Command>,
) -> End {
    let (mut reader, writer) = channel.split();
    loop {
        tokio::select! {
            msg = reader.wait() => match msg {
                Some(ChannelMsg::Data { data }) => entry.push_output(&data),
                Some(ChannelMsg::ExtendedData { data, .. }) => entry.push_output(&data),
                Some(ChannelMsg::Close) | None => return End::ByRemote,
                Some(_) => {}
            },
            cmd = commands.recv() => match cmd {
                Some(Command::Data(bytes)) => {
                    if writer.data(&bytes[..]).await.is_err() {
                        return End::ByRemote;
                    }
                }
                Some(Command::Resize { cols, rows }) => {
                    let _ = writer.window_change(cols, rows, 0, 0).await;
                }
                Some(Command::Close) | None => {
                    let _ = writer.close().await;
                    return End::ByUser;
                }
            },
        }
    }
}

async fn establish(options: &ConnectOptions) -> CoreResult<(Handle<Client>, Channel<Msg>)> {
    let key = load_key(&options.key_path)?;
    let target = &options.target;
    let limit = options.connect_timeout;
    let timed_out = || {
        CoreError::new(
            ErrorKind::Timeout,
            format!("Таймаут подключения к {}:{}", target.host, target.port),
        )
    };

    let addrs: Vec<_> = tokio::net::lookup_host((target.host.as_str(), target.port))
        .await
        .map_err(|_| {
            CoreError::new(
                ErrorKind::HostNotFound,
                format!("Хост не найден: {}", target.host),
            )
        })?
        .collect();

    let stream = timeout(limit, TcpStream::connect(&addrs[..]))
        .await
        .map_err(|_| timed_out())?
        .map_err(|e| map_connect_error(&e, &target.host, target.port))?;
    let _ = stream.set_nodelay(true);

    let config = Arc::new(client::Config {
        keepalive_interval: options.keepalive_interval,
        keepalive_max: 3,
        inactivity_timeout: None,
        nodelay: true,
        ..Default::default()
    });

    let mut handle = timeout(limit, client::connect_stream(config, stream, Client))
        .await
        .map_err(|_| timed_out())?
        .map_err(map_ssh_error)?;

    let hash = handle
        .best_supported_rsa_hash()
        .await
        .map_err(map_ssh_error)?
        .flatten();
    let auth = timeout(
        limit,
        handle.authenticate_publickey(
            target.user.clone(),
            PrivateKeyWithHashAlg::new(Arc::new(key), hash),
        ),
    )
    .await
    .map_err(|_| timed_out())?
    .map_err(map_ssh_error)?;
    if !auth.success() {
        return Err(CoreError::new(
            ErrorKind::AuthFailed,
            format!(
                "Сервер отклонил ключ для пользователя {}: доступ запрещён",
                target.user
            ),
        ));
    }

    let channel = handle.channel_open_session().await.map_err(map_ssh_error)?;
    channel
        .request_pty(false, TERM, options.cols, options.rows, 0, 0, &[])
        .await
        .map_err(map_ssh_error)?;
    channel.request_shell(false).await.map_err(map_ssh_error)?;

    Ok((handle, channel))
}

fn load_key(path: &Path) -> CoreResult<PrivateKey> {
    keys::load_secret_key(path, None).map_err(|e| match e {
        keys::Error::KeyIsEncrypted => CoreError::new(
            ErrorKind::KeyEncrypted,
            "Ключ защищён passphrase, такие ключи пока не поддерживаются",
        ),
        keys::Error::IO(io) if io.kind() == io::ErrorKind::NotFound => CoreError::new(
            ErrorKind::KeyNotFound,
            format!("Файл ключа не найден: {}", path.display()),
        ),
        other => CoreError::new(
            ErrorKind::KeyInvalid,
            format!("Не удалось прочитать ключ {}: {}", path.display(), other),
        ),
    })
}

fn map_connect_error(e: &io::Error, host: &str, port: u16) -> CoreError {
    match e.kind() {
        io::ErrorKind::ConnectionRefused => CoreError::new(
            ErrorKind::ConnectionRefused,
            format!("Соединение отклонено: {host}:{port} не принимает подключения"),
        ),
        io::ErrorKind::TimedOut => CoreError::new(
            ErrorKind::Timeout,
            format!("Таймаут подключения к {host}:{port}"),
        ),
        _ => CoreError::new(
            ErrorKind::Unreachable,
            format!("Хост {host}:{port} недоступен: {e}"),
        ),
    }
}

fn map_ssh_error(e: russh::Error) -> CoreError {
    match e {
        russh::Error::ConnectionTimeout
        | russh::Error::KeepaliveTimeout
        | russh::Error::InactivityTimeout => {
            CoreError::new(ErrorKind::Timeout, "Сервер не отвечает (таймаут)")
        }
        russh::Error::HUP | russh::Error::Disconnect => {
            CoreError::new(ErrorKind::Unreachable, "Сервер закрыл соединение")
        }
        russh::Error::IO(io) => {
            CoreError::new(ErrorKind::Unreachable, format!("Соединение прервано: {io}"))
        }
        russh::Error::NoCommonAlgo { .. } => CoreError::new(
            ErrorKind::Protocol,
            "Нет общих алгоритмов шифрования с сервером",
        ),
        other => CoreError::new(ErrorKind::Protocol, format!("Ошибка SSH: {other}")),
    }
}
