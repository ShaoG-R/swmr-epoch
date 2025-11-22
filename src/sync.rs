#[cfg(feature = "loom")]
pub use loom::cell::Cell;
#[cfg(not(feature = "loom"))]
pub use std::cell::Cell;

#[cfg(feature = "loom")]
pub use loom::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
#[cfg(not(feature = "loom"))]
pub use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

#[cfg(feature = "loom")]
pub use loom::sync::Arc;
#[cfg(not(feature = "loom"))]
pub use std::sync::Arc;

#[cfg(not(feature = "loom"))]
pub use antidote::Mutex;

#[cfg(feature = "loom")]
#[derive(Debug, Default)]
pub struct Mutex<T>(loom::sync::Mutex<T>);

#[cfg(feature = "loom")]
impl<T> Mutex<T> {
    pub fn new(t: T) -> Self {
        Self(loom::sync::Mutex::new(t))
    }

    pub fn lock(&self) -> loom::sync::MutexGuard<'_, T> {
        self.0.lock().unwrap()
    }
}
