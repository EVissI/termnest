use std::collections::VecDeque;
use std::sync::Arc;

/// Receiver of terminal output for one subscription (one terminal panel).
pub trait OutputSink: Send + Sync + 'static {
    /// Deliver a chunk of output. Returns `false` if the receiver is gone,
    /// in which case the subscription is dropped.
    fn send(&self, data: &[u8]) -> bool;
}

/// Recent output of a session plus its current subscribers.
///
/// The buffer lets a panel that subscribes later (or moves to another window)
/// replay what was printed before, so no output is lost.
pub(crate) struct OutputHub {
    buffer: VecDeque<u8>,
    capacity: usize,
    subscribers: Vec<(u64, Arc<dyn OutputSink>)>,
    next_subscriber: u64,
}

impl OutputHub {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            buffer: VecDeque::new(),
            capacity,
            subscribers: Vec::new(),
            next_subscriber: 1,
        }
    }

    pub(crate) fn push(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let keep = data.len().min(self.capacity);
        let overflow = (self.buffer.len() + keep).saturating_sub(self.capacity);
        self.buffer.drain(..overflow);
        self.buffer.extend(&data[data.len() - keep..]);

        self.subscribers.retain(|(_, sink)| sink.send(data));
    }

    /// Adds a subscriber and immediately replays the buffered output to it.
    pub(crate) fn subscribe(&mut self, sink: Arc<dyn OutputSink>) -> u64 {
        let id = self.next_subscriber;
        self.next_subscriber += 1;
        let (a, b) = self.buffer.as_slices();
        let mut alive = true;
        for part in [a, b] {
            if alive && !part.is_empty() {
                alive = sink.send(part);
            }
        }
        if alive {
            self.subscribers.push((id, sink));
        }
        id
    }

    pub(crate) fn unsubscribe(&mut self, id: u64) {
        self.subscribers.retain(|(sid, _)| *sid != id);
    }

    #[cfg(test)]
    pub(crate) fn buffered(&self) -> Vec<u8> {
        self.buffer.iter().copied().collect()
    }

    #[cfg(test)]
    pub(crate) fn subscriber_count(&self) -> usize {
        self.subscribers.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Collect {
        data: Mutex<Vec<u8>>,
        closed: std::sync::atomic::AtomicBool,
    }

    impl OutputSink for Collect {
        fn send(&self, data: &[u8]) -> bool {
            if self.closed.load(std::sync::atomic::Ordering::SeqCst) {
                return false;
            }
            self.data.lock().unwrap().extend_from_slice(data);
            true
        }
    }

    #[test]
    fn buffer_is_capped_and_keeps_tail() {
        let mut hub = OutputHub::new(8);
        hub.push(b"12345");
        hub.push(b"6789AB");
        assert_eq!(hub.buffered(), b"456789AB");
        hub.push(b"0123456789XYZ");
        assert_eq!(hub.buffered(), b"56789XYZ");
    }

    #[test]
    fn late_subscriber_gets_replay_then_live_data() {
        let mut hub = OutputHub::new(64);
        hub.push(b"hello ");
        let sink = Arc::new(Collect::default());
        hub.subscribe(sink.clone());
        hub.push(b"world");
        assert_eq!(&*sink.data.lock().unwrap(), b"hello world");
    }

    #[test]
    fn dead_and_unsubscribed_sinks_are_dropped() {
        let mut hub = OutputHub::new(64);
        let a = Arc::new(Collect::default());
        let b = Arc::new(Collect::default());
        let a_id = hub.subscribe(a.clone());
        hub.subscribe(b.clone());
        assert_eq!(hub.subscriber_count(), 2);

        b.closed.store(true, std::sync::atomic::Ordering::SeqCst);
        hub.push(b"x");
        assert_eq!(hub.subscriber_count(), 1);

        hub.unsubscribe(a_id);
        hub.push(b"y");
        assert_eq!(hub.subscriber_count(), 0);
        assert_eq!(&*a.data.lock().unwrap(), b"x");
    }
}
