//! Per-IP connection counting with RAII release. Shared by the SSH and telnet
//! transports.
//!
//! [`ConnTracker::acquire`] returns `Some(ConnSlot)` if a slot is free, and
//! the slot's `Drop` impl decrements the count automatically — there's no
//! manual release path to forget.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, PoisonError};

#[derive(Default)]
pub struct ConnTracker {
    counts: Mutex<HashMap<IpAddr, usize>>,
}

impl ConnTracker {
    /// Acquire a slot for `ip`. Returns `Some(guard)` on success — the count
    /// is automatically decremented when the guard is dropped. Returns `None`
    /// if the per-IP cap would be exceeded.
    pub fn acquire(self: &Arc<Self>, ip: IpAddr, per_ip: usize) -> Option<ConnSlot> {
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = counts.entry(ip).or_insert(0);
        if *entry >= per_ip {
            return None;
        }
        *entry += 1;
        Some(ConnSlot {
            tracker: Arc::clone(self),
            ip,
        })
    }

    fn release(&self, ip: IpAddr) {
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(c) = counts.get_mut(&ip) {
            *c = c.saturating_sub(1);
            if *c == 0 {
                counts.remove(&ip);
            }
        }
    }
}

/// RAII guard for a connection slot. Releases on drop.
pub struct ConnSlot {
    tracker: Arc<ConnTracker>,
    ip: IpAddr,
}

impl Drop for ConnSlot {
    fn drop(&mut self) {
        self.tracker.release(self.ip);
    }
}
