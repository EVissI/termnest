//! Shared SSH connection logic: host key verification, key loading with passphrase
//! prompts, authentication and one-shot command execution.

use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client::{self, Handle};
use russh::keys::{self, PrivateKey, PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate};
use russh::ChannelMsg;
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::error::{CoreError, CoreResult, ErrorKind, HostKeyInfo};
use crate::hostkeys::{fingerprint, HostKeyStatus, KnownHosts};
use crate::prompt::{self, PromptReply, PromptRequest, Prompter};
use crate::secret::{PassphraseCache, SecretString};
use crate::target::Target;

const TCP_TIMEOUT: Duration = Duration::from_secs(15);
/// Covers the handshake including the time the user spends on the host key dialog.
/// The server's LoginGraceTime (120 s by default) bounds it anyway.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(150);
const AUTH_TIMEOUT: Duration = Duration::from_secs(30);

/// Everything a connection needs besides the target.
pub struct SshContext {
    pub prompter: Arc<dyn Prompter>,
    pub known_hosts: Arc<KnownHosts>,
    pub passphrases: Arc<PassphraseCache>,
    keepalive_secs: std::sync::atomic::AtomicU32,
}

impl SshContext {
    pub fn new(
        prompter: Arc<dyn Prompter>,
        known_hosts: Arc<KnownHosts>,
        passphrases: Arc<PassphraseCache>,
        keepalive_secs: u32,
    ) -> Self {
        Self {
            prompter,
            known_hosts,
            passphrases,
            keepalive_secs: keepalive_secs.into(),
        }
    }

    pub fn set_keepalive_secs(&self, secs: u32) {
        self.keepalive_secs
            .store(secs, std::sync::atomic::Ordering::Relaxed);
    }

    fn keepalive(&self) -> Option<Duration> {
        match self
            .keepalive_secs
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            0 => None,
            s => Some(Duration::from_secs(s.into())),
        }
    }
}

pub struct Client {
    host: String,
    port: u16,
    known_hosts: Arc<KnownHosts>,
    prompter: Arc<dyn Prompter>,
    /// Why the host key was refused, reported instead of russh's generic error.
    verdict: Arc<Mutex<Option<CoreError>>>,
}

impl Client {
    fn refuse(&self, err: CoreError) -> Result<bool, russh::Error> {
        *self.verdict.lock().unwrap_or_else(|e| e.into_inner()) = Some(err);
        Ok(false)
    }
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let key: PublicKey = match server_key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            PublicKeyOrCertificate::Certificate(cert) => PublicKey::from(cert.public_key().clone()),
        };
        let info = HostKeyInfo {
            host: self.host.clone(),
            port: self.port,
            key_type: key.algorithm().to_string(),
            fingerprint: fingerprint(&key),
            known_fingerprint: None,
        };
        match self.known_hosts.check(&self.host, self.port, &key) {
            HostKeyStatus::Trusted => Ok(true),
            HostKeyStatus::Changed { known_fingerprint } => {
                let mut err = CoreError::new(
                    ErrorKind::HostKeyChanged,
                    format!(
                        "Ключ сервера {}:{} изменился! Возможна атака «человек посередине». Подключение заблокировано.",
                        self.host, self.port
                    ),
                );
                err.host_key = Some(Box::new(HostKeyInfo {
                    known_fingerprint: Some(known_fingerprint),
                    ..info
                }));
                self.refuse(err)
            }
            HostKeyStatus::Unknown => {
                let reply = prompt::ask(
                    self.prompter.as_ref(),
                    PromptRequest::HostKeyUnknown {
                        host: info.host.clone(),
                        port: info.port,
                        key_type: info.key_type.clone(),
                        fingerprint: info.fingerprint.clone(),
                    },
                )
                .await;
                if matches!(reply, PromptReply::Confirm) {
                    if let Err(e) = self.known_hosts.add(&self.host, self.port, &key) {
                        log::warn!("failed to save host key: {e}");
                    }
                    Ok(true)
                } else {
                    let mut err = CoreError::new(
                        ErrorKind::HostKeyRejected,
                        "Ключ сервера не принят, подключение отменено",
                    );
                    err.host_key = Some(Box::new(info));
                    self.refuse(err)
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
pub enum Auth<'a> {
    Key(&'a Path),
    Password(&'a SecretString),
}

/// Loads a private key, asking for the passphrase if the key is encrypted.
pub async fn load_private_key(path: &Path, ctx: &SshContext) -> CoreResult<PrivateKey> {
    let owned = path.to_path_buf();
    match blocking_load(owned.clone(), None).await {
        Err(e) if e.kind == ErrorKind::KeyEncrypted => {}
        other => return other,
    }

    if let Some(cached) = ctx.passphrases.get(path) {
        match blocking_load(owned.clone(), Some(cached)).await {
            Ok(k) => return Ok(k),
            Err(_) => ctx.passphrases.remove(path),
        }
    }

    let mut retry = false;
    loop {
        let reply = prompt::ask(
            ctx.prompter.as_ref(),
            PromptRequest::Passphrase {
                key_path: path.display().to_string(),
                retry,
            },
        )
        .await;
        let PromptReply::Passphrase { value, remember } = reply else {
            return Err(CoreError::new(
                ErrorKind::KeyEncrypted,
                "Ввод passphrase отменён",
            ));
        };
        match blocking_load(owned.clone(), Some(value.clone())).await {
            Ok(key) => {
                if remember {
                    ctx.passphrases.insert(path, value);
                }
                return Ok(key);
            }
            Err(e) if e.kind == ErrorKind::KeyEncrypted || e.kind == ErrorKind::KeyInvalid => {
                retry = true;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Key decryption (bcrypt-pbkdf) is CPU-heavy, so it runs off the async runtime.
async fn blocking_load(
    path: std::path::PathBuf,
    passphrase: Option<SecretString>,
) -> CoreResult<PrivateKey> {
    tokio::task::spawn_blocking(move || {
        keys::load_secret_key(&path, passphrase.as_ref().map(|p| p.expose())).map_err(|e| match e {
            keys::Error::KeyIsEncrypted => {
                CoreError::new(ErrorKind::KeyEncrypted, "Ключ защищён passphrase")
            }
            keys::Error::IO(io) if io.kind() == io::ErrorKind::NotFound => CoreError::new(
                ErrorKind::KeyNotFound,
                format!("Файл ключа не найден: {}", path.display()),
            ),
            other => CoreError::new(
                ErrorKind::KeyInvalid,
                format!("Не удалось прочитать ключ {}: {}", path.display(), other),
            ),
        })
    })
    .await
    .map_err(|e| CoreError::new(ErrorKind::KeyInvalid, format!("Ошибка чтения ключа: {e}")))?
}

/// Opens a TCP connection, performs the SSH handshake with host key verification
/// and authenticates.
pub async fn connect(
    target: &Target,
    auth: Auth<'_>,
    ctx: &SshContext,
) -> CoreResult<Handle<Client>> {
    // Load the key before touching the network, so a passphrase dialog does not
    // eat into the server's login grace time.
    let key = match auth {
        Auth::Key(path) => Some(load_private_key(path, ctx).await?),
        Auth::Password(_) => None,
    };

    let mut handle = handshake(target, ctx).await?;
    let timed_out = || {
        CoreError::new(
            ErrorKind::Timeout,
            format!("Таймаут подключения к {}:{}", target.host, target.port),
        )
    };

    let user = target.user.clone();
    let result = match (auth, key) {
        (Auth::Key(_), Some(key)) => {
            let hash = handle
                .best_supported_rsa_hash()
                .await
                .map_err(map_ssh_error)?
                .flatten();
            timeout(
                AUTH_TIMEOUT,
                handle
                    .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), hash)),
            )
            .await
        }
        (Auth::Password(password), _) => {
            timeout(
                AUTH_TIMEOUT,
                handle.authenticate_password(user, password.expose()),
            )
            .await
        }
        (Auth::Key(_), None) => unreachable!("key is loaded for key auth"),
    };
    let auth_result = result.map_err(|_| timed_out())?.map_err(map_ssh_error)?;
    if !auth_result.success() {
        let what = match auth {
            Auth::Key(_) => "Сервер отклонил ключ",
            Auth::Password(_) => "Неверный пароль или вход по паролю запрещён",
        };
        return Err(CoreError::new(
            ErrorKind::AuthFailed,
            format!("{what} (пользователь {})", target.user),
        ));
    }
    Ok(handle)
}

/// TCP connection and SSH handshake with host key verification, no authentication.
async fn handshake(target: &Target, ctx: &SshContext) -> CoreResult<Handle<Client>> {
    let host = target.host.as_str();
    let port = target.port;
    let timed_out = || {
        CoreError::new(
            ErrorKind::Timeout,
            format!("Таймаут подключения к {host}:{port}"),
        )
    };

    let addrs: Vec<_> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| CoreError::new(ErrorKind::HostNotFound, format!("Хост не найден: {host}")))?
        .collect();
    let stream = timeout(TCP_TIMEOUT, TcpStream::connect(&addrs[..]))
        .await
        .map_err(|_| timed_out())?
        .map_err(|e| map_connect_error(&e, host, port))?;
    let _ = stream.set_nodelay(true);

    let config = Arc::new(client::Config {
        keepalive_interval: ctx.keepalive(),
        keepalive_max: 3,
        inactivity_timeout: None,
        nodelay: true,
        ..Default::default()
    });
    let verdict = Arc::new(Mutex::new(None));
    let handler = Client {
        host: host.to_owned(),
        port,
        known_hosts: ctx.known_hosts.clone(),
        prompter: ctx.prompter.clone(),
        verdict: verdict.clone(),
    };

    let handshake = timeout(
        HANDSHAKE_TIMEOUT,
        client::connect_stream(config, stream, handler),
    )
    .await
    .map_err(|_| timed_out())?;
    match handshake {
        Ok(h) => Ok(h),
        Err(e) => {
            if let Some(v) = verdict.lock().unwrap_or_else(|e| e.into_inner()).take() {
                return Err(v);
            }
            Err(map_ssh_error(e))
        }
    }
}

/// Authentication methods the server offers to `target.user` before any credentials
/// (the reply to the "none" method), e.g. `publickey`, `password`.
pub async fn auth_methods(target: &Target, ctx: &SshContext) -> CoreResult<Vec<String>> {
    let mut handle = handshake(target, ctx).await?;
    let reply = timeout(AUTH_TIMEOUT, handle.authenticate_none(target.user.clone()))
        .await
        .map_err(|_| {
            CoreError::new(
                ErrorKind::Timeout,
                format!("Таймаут подключения к {}:{}", target.host, target.port),
            )
        })?
        .map_err(map_ssh_error);
    disconnect(&handle).await;
    Ok(match reply? {
        // The server let us in without any credentials.
        client::AuthResult::Success => vec!["none".to_owned()],
        client::AuthResult::Failure {
            remaining_methods, ..
        } => remaining_methods
            .iter()
            .map(|m| <&str>::from(m).to_owned())
            .collect(),
    })
}

/// Whether the offered methods let someone in without a key.
pub fn allows_password(methods: &[String]) -> bool {
    methods
        .iter()
        .any(|m| matches!(m.as_str(), "password" | "keyboard-interactive" | "none"))
}

pub struct ExecOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<u32>,
}

/// Runs a command on a new exec channel, optionally feeding `stdin`.
pub async fn exec(
    handle: &Handle<Client>,
    command: &str,
    stdin: Option<&[u8]>,
) -> CoreResult<ExecOutput> {
    let mut channel = handle.channel_open_session().await.map_err(map_ssh_error)?;
    channel.exec(true, command).await.map_err(map_ssh_error)?;
    if let Some(data) = stdin {
        channel.data(data).await.map_err(map_ssh_error)?;
    }
    channel.eof().await.map_err(map_ssh_error)?;

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_code = None;
    let collect = async {
        while let Some(msg) = channel.wait().await {
            match msg {
                ChannelMsg::Data { data } => stdout.extend_from_slice(&data),
                ChannelMsg::ExtendedData { data, .. } => stderr.extend_from_slice(&data),
                ChannelMsg::ExitStatus { exit_status } => exit_code = Some(exit_status),
                ChannelMsg::Close => break,
                _ => {}
            }
        }
    };
    timeout(Duration::from_secs(60), collect)
        .await
        .map_err(|_| {
            CoreError::new(
                ErrorKind::Timeout,
                "Команда на сервере не завершилась вовремя",
            )
        })?;
    Ok(ExecOutput {
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        exit_code,
    })
}

pub async fn disconnect(handle: &Handle<Client>) {
    let _ = handle
        .disconnect(russh::Disconnect::ByApplication, "", "en")
        .await;
}

pub fn map_connect_error(e: &io::Error, host: &str, port: u16) -> CoreError {
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

pub fn map_ssh_error(e: russh::Error) -> CoreError {
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
