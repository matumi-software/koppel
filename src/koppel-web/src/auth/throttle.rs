//! Slows down guessing the password, as 0.1.x's `UiLoginThrottle`.
//!
//! A try is booked before the password is looked at and counts as a failure until it turns out
//! right, so parallel guesses cannot all slip in before the first is booked. After five booked tries
//! a caller is locked for a minute, and each further one doubles that, up to an hour. A right
//! password clears the caller. The lock applies before the password is looked at, so a right guess
//! during it is refused too.
//!
//! Two more limits sit beside the per-caller lock. A global budget counts every booked try from any
//! caller, so a flood of source addresses cannot guess faster than the budget allows: once it is
//! spent every caller is refused until its window is over. And the table of callers is capped: when
//! it is full the stale callers go first, then the unlocked ones not seen for longest. A caller that
//! is locked is never evicted, so a flood of new callers cannot free a locked one; when nothing can
//! go, a new caller is not tracked and only the global budget holds it.

use std::collections::HashMap;
use std::time::{Duration, Instant};

pub(crate) const FREE_FAILURES: u32 = 5;
const MAX_KEYS: usize = 2048;
const FIRST_LOCK: Duration = Duration::from_secs(60);
const MAX_LOCK: Duration = Duration::from_secs(3600);
/// A caller not seen for this long starts again from nothing.
const FORGET: Duration = Duration::from_secs(2 * 3600);
/// Booked tries, from all callers together, before every caller is refused for the rest of the
/// window.
pub(crate) const GLOBAL_BUDGET: u32 = 30;
pub(crate) const GLOBAL_WINDOW: Duration = Duration::from_secs(600);

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

/// The tries booked in the current window, from every caller.
#[derive(Debug, Clone, Copy)]
struct Budget {
    started: Instant,
    used: u32,
}

#[derive(Default)]
pub(crate) struct Throttle {
    entries: HashMap<String, Entry>,
    budget: Option<Budget>,
}

impl Throttle {
    /// How long until `key` may try, or `None` when it may: a read, with nothing booked. It is the
    /// cheap check to make before taking a hash slot.
    pub(crate) fn check(&self, key: &str, now: Instant) -> Option<Duration> {
        self.budget_wait(now)
            .or_else(|| self.entries.get(key).and_then(|e| remaining(e, now)))
    }

    fn budget_wait(&self, now: Instant) -> Option<Duration> {
        let budget = self.budget?;
        if budget.used < GLOBAL_BUDGET {
            return None;
        }
        (budget.started + GLOBAL_WINDOW)
            .checked_duration_since(now)
            .filter(|left| !left.is_zero())
    }

    /// Books a try as a failure unless the caller is locked or the global budget is spent. Call
    /// [`Self::clear`] when it was right.
    pub(crate) fn reserve(&mut self, key: &str, now: Instant) -> Reservation {
        let known = self.entries.get(key).copied();
        let wait = self
            .budget_wait(now)
            .or_else(|| known.as_ref().and_then(|e| remaining(e, now)));
        if let Some(wait) = wait {
            return Reservation {
                granted: false,
                lock: wait,
                failures: known.map_or(0, |e| e.failures),
            };
        }
        self.book_global(now);
        let entry = match known {
            Some(entry) => entry,
            None if self.make_room(now) => Entry {
                failures: 0,
                locked_until: None,
                last: now,
            },
            // Every place is held by a locked caller. This one goes untracked, and the budget
            // booked above still bounds it.
            None => {
                return Reservation {
                    granted: true,
                    lock: Duration::ZERO,
                    failures: 1,
                };
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

    fn book_global(&mut self, now: Instant) {
        match &mut self.budget {
            Some(budget) if now.duration_since(budget.started) < GLOBAL_WINDOW => budget.used += 1,
            slot => {
                *slot = Some(Budget {
                    started: now,
                    used: 1,
                });
            }
        }
    }

    /// The try was right: the caller starts again, and the try goes back to the global budget.
    pub(crate) fn clear(&mut self, key: &str) {
        self.entries.remove(key);
        if let Some(budget) = &mut self.budget {
            budget.used = budget.used.saturating_sub(1);
        }
    }

    /// Makes room for one more caller. False when the table is full of locked callers.
    fn make_room(&mut self, now: Instant) -> bool {
        if self.entries.len() < MAX_KEYS {
            return true;
        }
        self.entries.retain(|_, e| {
            now.duration_since(e.last) <= FORGET || e.locked_until.is_some_and(|until| until > now)
        });
        // Still full: the unlocked caller unseen for longest goes. A locked one never does.
        while self.entries.len() >= MAX_KEYS {
            let Some(oldest) = self
                .entries
                .iter()
                .filter(|(_, e)| remaining(e, now).is_none())
                .min_by_key(|(_, e)| e.last)
                .map(|(k, _)| k.clone())
            else {
                return false;
            };
            self.entries.remove(&oldest);
        }
        true
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

    fn flood(throttle: &mut Throttle, now: Instant, locked: Option<Duration>) {
        for i in 0..MAX_KEYS {
            throttle.entries.insert(
                format!("flood{i}"),
                Entry {
                    failures: 1,
                    locked_until: locked.map(|lock| now + lock),
                    last: now,
                },
            );
        }
    }

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

    #[test]
    fn a_locked_caller_is_not_evicted_by_a_flood_of_new_ones() {
        let mut throttle = Throttle::default();
        let now = Instant::now();
        for _ in 0..FREE_FAILURES {
            throttle.reserve("locked", now);
        }
        flood(&mut throttle, now, None);
        for _ in 0..3 {
            assert!(throttle.make_room(now), "unlocked callers make room");
        }
        assert!(throttle.entries.len() < MAX_KEYS);
        assert!(
            throttle.check("locked", now).is_some(),
            "the locked caller is still locked"
        );
    }

    #[test]
    fn a_table_full_of_locked_callers_does_not_track_a_new_one() {
        let mut throttle = Throttle::default();
        let now = Instant::now();
        flood(&mut throttle, now, Some(MAX_LOCK));
        let new = throttle.reserve("new", now);
        assert!(new.granted);
        assert_eq!(throttle.entries.len(), MAX_KEYS);
        assert!(!throttle.entries.contains_key("new"));
    }

    #[test]
    fn the_global_budget_refuses_every_caller_until_its_window_is_over() {
        let mut throttle = Throttle::default();
        let now = Instant::now();
        for i in 0..GLOBAL_BUDGET {
            assert!(throttle.reserve(&format!("caller{i}"), now).granted, "{i}");
        }
        assert!(throttle.check("fresh", now).is_some());
        let refused = throttle.reserve("fresh", now + Duration::from_secs(1));
        assert!(!refused.granted);
        assert_eq!(refused.lock, GLOBAL_WINDOW - Duration::from_secs(1));
        assert!(
            throttle.reserve("fresh", now + GLOBAL_WINDOW).granted,
            "a new window starts"
        );
    }

    #[test]
    fn right_tries_do_not_spend_the_global_budget() {
        let mut throttle = Throttle::default();
        let now = Instant::now();
        for _ in 0..GLOBAL_BUDGET * 3 {
            assert!(throttle.reserve("admin", now).granted);
            throttle.clear("admin");
        }
        assert!(throttle.check("admin", now).is_none());
    }

    #[test]
    fn a_check_books_nothing() {
        let mut throttle = Throttle::default();
        let now = Instant::now();
        for _ in 0..100 {
            assert!(throttle.check("a", now).is_none());
        }
        assert_eq!(throttle.reserve("a", now).failures, 1);
    }
}
