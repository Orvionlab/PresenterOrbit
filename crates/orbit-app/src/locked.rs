//! Locking that survives a panic on another thread. The guarded values are plain data (handles, handlers, the
//! last error), so a poisoned lock must not turn one thread's panic into a panic on the UI or output threads.

use std::sync::{Mutex, MutexGuard, PoisonError};

pub trait Locked<T> {
    fn locked(&self) -> MutexGuard<'_, T>;
}

impl<T> Locked<T> for Mutex<T> {
    fn locked(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
