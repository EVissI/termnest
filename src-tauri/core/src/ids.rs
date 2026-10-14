use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Short unique id for stored entities (hosts, snippets): time-based plus a counter
/// and some randomness, rendered as hex.
pub fn new_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let r: u32 = rand::random();
    format!("{:x}{:x}{:08x}", nanos, n & 0xfff, r)
}

#[cfg(test)]
mod tests {
    #[test]
    fn ids_are_unique() {
        let ids: std::collections::HashSet<_> = (0..1000).map(|_| super::new_id()).collect();
        assert_eq!(ids.len(), 1000);
    }
}
