//! End-to-end test against a real SSH server. Skipped unless configured:
//!
//! TERMNEST_TEST_TARGET=user@host:port TERMNEST_TEST_KEY=/path/to/key cargo test -p termnest-core --test live_ssh

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use termnest_core::session::ConnectOptions;
use termnest_core::{CoreError, OutputSink, SessionId, SessionManager, SessionState, StateSink};

#[derive(Default)]
struct States(Mutex<Vec<SessionState>>);

impl StateSink for States {
    fn state_changed(&self, _id: SessionId, state: SessionState, error: Option<&CoreError>) {
        if let Some(e) = error {
            eprintln!("state {state:?}: {e}");
        }
        self.0.lock().unwrap().push(state);
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

async fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    for _ in 0..300 {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

#[tokio::test]
async fn shell_roundtrip() {
    let (Ok(target), Ok(key)) = (
        std::env::var("TERMNEST_TEST_TARGET"),
        std::env::var("TERMNEST_TEST_KEY"),
    ) else {
        eprintln!("TERMNEST_TEST_TARGET/TERMNEST_TEST_KEY not set, skipping");
        return;
    };

    let states = Arc::new(States::default());
    let manager = SessionManager::new(states.clone());
    let id = manager.open(ConnectOptions::new(
        target.parse().unwrap(),
        PathBuf::from(key),
        100,
        30,
    ));

    wait_until("connected", || {
        states.0.lock().unwrap().contains(&SessionState::Connected)
    })
    .await;

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

    manager.write(id, b"exit\n".to_vec()).unwrap();
    wait_until("disconnect", || {
        states
            .0
            .lock()
            .unwrap()
            .contains(&SessionState::Disconnected)
    })
    .await;
}
