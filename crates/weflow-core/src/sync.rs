//! Lock acquisition that treats a poisoned `std::sync` lock as fatal.
//!
//! A lock is poisoned when a thread panicked while holding it, so the guarded state may be half updated.
//! Nothing in WeFlow can repair such state, so every lock is acquired through these methods, which panic
//! with one message instead of each call site repeating `.expect("lock poisoned")`.

use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

const POISONED: &str = "lock poisoned: a thread panicked while holding it";

/// Poison-fatal acquisition for [`Mutex`].
pub trait MutexExt<T: ?Sized> {
    /// Locks the mutex, blocking until it is free.
    ///
    /// # Panics
    ///
    /// Panics if another thread panicked while holding the lock.
    fn lock_or_panic(&self) -> MutexGuard<'_, T>;

    /// Consumes the mutex and returns the guarded value.
    ///
    /// # Panics
    ///
    /// Panics if another thread panicked while holding the lock.
    fn into_inner_or_panic(self) -> T
    where
        Self: Sized,
        T: Sized;
}

#[expect(clippy::expect_used, reason = "a poisoned lock guards state that cannot be trusted")]
impl<T: ?Sized> MutexExt<T> for Mutex<T> {
    fn lock_or_panic(&self) -> MutexGuard<'_, T> {
        self.lock().expect(POISONED)
    }

    fn into_inner_or_panic(self) -> T
    where
        T: Sized,
    {
        self.into_inner().expect(POISONED)
    }
}

/// Poison-fatal acquisition for [`RwLock`].
pub trait RwLockExt<T: ?Sized> {
    /// Locks for shared reading, blocking until no writer holds the lock.
    ///
    /// # Panics
    ///
    /// Panics if another thread panicked while holding the lock for writing.
    fn read_or_panic(&self) -> RwLockReadGuard<'_, T>;

    /// Locks for exclusive writing, blocking until no reader or writer holds the lock.
    ///
    /// # Panics
    ///
    /// Panics if another thread panicked while holding the lock for writing.
    fn write_or_panic(&self) -> RwLockWriteGuard<'_, T>;
}

#[expect(clippy::expect_used, reason = "a poisoned lock guards state that cannot be trusted")]
impl<T: ?Sized> RwLockExt<T> for RwLock<T> {
    fn read_or_panic(&self) -> RwLockReadGuard<'_, T> {
        self.read().expect(POISONED)
    }

    fn write_or_panic(&self) -> RwLockWriteGuard<'_, T> {
        self.write().expect(POISONED)
    }
}
