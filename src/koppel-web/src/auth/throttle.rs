//! Slows down guessing the password, as 0.1.x's `UiLoginThrottle`.
//!
//! A try is booked before the password is looked at and counts as a failure until it turns out
//! right, so parallel guesses cannot all slip in before the first is booked. After five booked tries
//! a caller is locked for a minute, and each further one doubles that, up to an hour. A right
//! password clears the caller. The lock applies before the password is looked at, so a right guess
//! during it is refused too. The table is capped: when it is full the stale callers go first, then
//! the ones not seen for longest.

use std::collections::HashMap;
use std::time::{Duration, Instant};

pub(crate) const FREE_FAILURES: u32 = 5;
const MAX_KEYS: usize = 2048;
const FIRST_LOCK: Duration = Duration::from_secs(60);
const MAX_LOCK: Duration = Duration::from_secs(3600);
/// A caller not seen for this long starts again from nothing.
const FORGET: Duration = Duration::from_secs(2 * 3600);

/// What [`Throttle::reserve`] decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Reservation {
    /// False while the caller is locked: the try must not be looked at.
    pub granted: bool,
    /// Refused: how long until another try. Granted: the lock this try engaged, or zero.
    pub lock: Duration,
    /// Granted: how many tries count against the caller now, this one included.
    pub failures: u32,
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    failures: u32,
    locked_until: Option<Instant>,
    last: Instant,
}

#[derive(Default)]
pub(crate) struct Throttle {
    entries: HashMap<String, Entry>,
}

impl Throttle {
    /// Books a try as a failure unless the caller is locked. Call [`Self::clear`] when it was right.
    pub(crate) fn reserve(&mut self, key: &str, now: Instant) -> Reservation {
        let entry = match self.entries.get(key) {
            Some(entry) => {
                if let Some(wait) = remaining(entry, now) {
                    return Reservation {
                        granted: false,
                        lock: wait,
                        failures: entry.failures,
                    };
                }
                *entry
            }
            None => {
                self.make_room(now);
                Entry {
                    failures: 0,
                    locked_until: None,
                    last: now,
                }
            }
        };
        let next = next(entry, now);
        self.entries.insert(key.to_owned(), next);
        Reservation {
            granted: true,
            lock: remaining(&next, now).unwrap_or_default(),
            failures: next.failures,
        }
    }

    pub(crate) fn clear(&mut self, key: &str) {
        self.entries.remove(key);
    }

    fn make_room(&mut self, now: Instant) {
        if self.entries.len() < MAX_KEYS {
            return;
        }
        self.entries.retain(|_, e| {
            now.duration_since(e.last) <= FORGET || e.locked_until.is_some_and(|until| until > now)
        });
        // Still full: the caller unseen for longest goes, so a flood of new callers cannot grow it.
        while self.entries.len() >= MAX_KEYS {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.last)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            self.entries.remove(&oldest);
        }
    }
}

fn remaining(entry: &Entry, now: Instant) -> Option<Duration> {
    entry
        .locked_until
        .and_then(|until| until.checked_duration_since(now))
        .filter(|left| !left.is_zero())
}

fn next(entry: Entry, now: Instant) -> Entry {
    let failures = if now.duration_since(entry.last) > FORGET {
        1
    } else {
        entry.failures + 1
    };
    if failures < FREE_FAILURES {
        return Entry {
            failures,
            locked_until: None,
            last: now,
        };
    }
    let doublings = (failures - FREE_FAILURES).min(20);
    let lock = FIRST_LOCK.saturating_mul(1 << doublings).min(MAX_LOCK);
    Entry {
        failures,
        locked_until: Some(now + lock),
        last: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fifth_failure_locks_for_a_minute_and_each_after_doubles() {
        let mut throttle = Throttle::default();
        let mut now = Instant::now();
        for i in 1..FREE_FAILURES {
            let r = throttle.reserve("a", now);
            assert!(r.granted && r.lock.is_zero() && r.failures == i);
        }
        let fifth = throttle.reserve("a", now);
        assert!(fifth.granted);
        assert_eq!(fifth.lock, FIRST_LOCK);
        let refused = throttle.reserve("a", now + Duration::from_secs(30));
        assert!(!refused.granted);
        assert_eq!(refused.lock, Duration::from_secs(30));

        now += FIRST_LOCK;
        let sixth = throttle.reserve("a", now);
        assert_eq!(sixth.lock, FIRST_LOCK * 2);
        assert!(throttle.reserve("b", now).granted, "callers are apart");
        throttle.clear("a");
        assert!(throttle.reserve("a", now).granted);
    }

    #[test]
    fn the_lock_stops_at_an_hour() {
        let entry = Entry {
            failures: 40,
            locked_until: None,
            last: Instant::now(),
        };
        let next = next(entry, entry.last);
        assert_eq!(next.locked_until.unwrap() - entry.last, MAX_LOCK);
    }
}
