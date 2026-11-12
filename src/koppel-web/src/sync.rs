//! Locks that a panic elsewhere does not poison for good. Every lock in this crate guards a value
//! that is replaced or edited whole, so what a panicking holder left is still a value the next
//! caller can use, and a poisoned lock would only turn one panic into many.

use std::sync::{Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// `lock` that takes the guard of a poisoned mutex instead of failing.
pub(crate) trait Unpoisoned<T> {
    fn locked(&self) -> MutexGuard<'_, T>;
}

impl<T> Unpoisoned<T> for Mutex<T> {
    fn locked(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The same for a read-write lock, which has two guards.
pub(crate) trait UnpoisonedRw<T> {
    fn read_locked(&self) -> RwLockReadGuard<'_, T>;
    fn write_locked(&self) -> RwLockWriteGuard<'_, T>;
}

impl<T> UnpoisonedRw<T> for RwLock<T> {
    fn read_locked(&self) -> RwLockReadGuard<'_, T> {
        self.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write_locked(&self) -> RwLockWriteGuard<'_, T> {
        self.write().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn a_lock_a_panic_poisoned_still_opens() {
        let mutex = Arc::new(Mutex::new(1));
        let held = mutex.clone();
        let _ = std::thread::spawn(move || {
            let _guard = held.lock().unwrap();
            panic!("a holder panics");
        })
        .join();
        assert!(mutex.is_poisoned());
        assert_eq!(*mutex.locked(), 1);

        let lock = Arc::new(RwLock::new(2));
        let held = lock.clone();
        let _ = std::thread::spawn(move || {
            let _guard = held.write().unwrap();
            panic!("a holder panics");
        })
        .join();
        assert_eq!(*lock.read_locked(), 2);
        *lock.write_locked() = 3;
        assert_eq!(*lock.read_locked(), 3);
    }
}
