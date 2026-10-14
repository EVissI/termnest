//! End-to-end tests against a real SSH server. Skipped unless configured:
//!
//! TERMNEST_TEST_TARGET=user@host:port TERMNEST_TEST_KEY=/path/to/key cargo test -p termnest-core --test live_ssh

use std::sync::{Arc, Mutex};
use std::time::Duration;

use termnest_core::hostkeys::KnownHosts;
use termnest_core::prompt::{PromptReply, PromptRequest, Prompter};
use termnest_core::secret::PassphraseCache;
use termnest_core::ssh::SshContext;
use termnest_core::{
    ConnectSpec, OutputSink, SessionManager, SessionState, SessionStatus, StateSink, Target,
};
use tokio::sync::oneshot;

#[derive(Default)]
struct States(Mutex<Vec<SessionState>>);

impl StateSink for States {
    fn state_changed(&self, s: &SessionStatus) {
        if let Some(e) = &s.error {
            eprintln!("state {:?}: {}", s.state, e);
        }
        self.0.lock().unwrap().push(s.state);
    }
}

#[derive(Default)]
struct Output(Mutex<Vec<u8>>);

impl OutputSink for Output {
    fn send(&self, data: &[u8]) -> bool {
        self.0.lock().unwrap().extend_from_slice(data);
        true
    }
}

impl Output {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

/// Trusts every unknown host key and counts how often it was asked.
#[derive(Default)]
struct TrustAll(Mutex<u32>);

impl Prompter for TrustAll {
    fn ask(&self, request: PromptRequest) -> oneshot::Receiver<PromptReply> {
        let (tx, rx) = oneshot::channel();
        let reply = match request {
            PromptRequest::HostKeyUnknown { .. } => {
                *self.0.lock().unwrap() += 1;
                PromptReply::Confirm
            }
            _ => PromptReply::Cancel,
        };
        let _ = tx.send(reply);
        rx
    }
}

async fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    for _ in 0..300 {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

fn config() -> Option<(Target, String)> {
    let target = std::env::var("TERMNEST_TEST_TARGET").ok()?;
    let key = std::env::var("TERMNEST_TEST_KEY").ok()?;
    Some((target.parse().unwrap(), key))
}

#[tokio::test]
async fn shell_roundtrip() {
    let Some((target, key)) = config() else {
        eprintln!("TERMNEST_TEST_TARGET/TERMNEST_TEST_KEY not set, skipping");
        return;
    };
    let dir = std::env::temp_dir().join(format!("termnest-live-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let prompter = Arc::new(TrustAll::default());
    let ctx = Arc::new(SshContext::new(
        prompter.clone(),
        Arc::new(KnownHosts::load(dir.join("known_hosts")).unwrap()),
        Arc::new(PassphraseCache::default()),
        30,
    ));
    let states = Arc::new(States::default());
    let manager = SessionManager::new(states.clone(), ctx);
    let spec = ConnectSpec::from_target(&target, key, None);
    let id = manager.open(spec.clone(), 100, 30);

    wait_until("connected", || {
        states.0.lock().unwrap().contains(&SessionState::Connected)
    })
    .await;
    assert_eq!(
        *prompter.0.lock().unwrap(),
        1,
        "unknown host key must be asked once"
    );

    // Subscribe after some output was produced: the prompt must be replayed.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let out = Arc::new(Output::default());
    manager.subscribe(id, out.clone()).unwrap();
    assert!(!out.text().is_empty(), "buffered output was not replayed");

    manager.resize(id, 132, 40).unwrap();
    manager
        .write(id, b"stty size; echo marker-$((40+2))\n".to_vec())
        .unwrap();
    wait_until("command output", || out.text().contains("marker-42")).await;
    assert!(
        out.text().contains("40 132"),
        "pty size not applied: {}",
        out.text()
    );

    // `exit` ends the shell: the session goes to Disconnected and does not reconnect.
    manager.write(id, b"exit\n".to_vec()).unwrap();
    wait_until("disconnect", || {
        states
            .0
            .lock()
            .unwrap()
            .contains(&SessionState::Disconnected)
    })
    .await;
    assert_eq!(
        manager.status(id).unwrap().state,
        SessionState::Disconnected
    );

    // Retry reconnects with the now known host key: no second prompt.
    manager.retry(id).unwrap();
    wait_until("reconnected", || {
        manager.status(id).map(|s| s.state) == Ok(SessionState::Connected)
    })
    .await;
    assert_eq!(*prompter.0.lock().unwrap(), 1);

    // A separate exec channel on the same connection.
    let out = manager.exec(id, "echo exec-ok").await.unwrap();
    assert_eq!(out.stdout.trim(), "exec-ok");

    // SFTP over the same connection.
    let sftp = manager.sftp(id).await.unwrap();
    let home = sftp.canonicalize(".").await.unwrap();
    assert!(home.starts_with('/'));

    manager.close(id);
    let _ = std::fs::remove_dir_all(dir);
}
