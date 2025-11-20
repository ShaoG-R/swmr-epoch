#[cfg(loom)]
pub use loom::cell::Cell;
#[cfg(not(loom))]
pub use std::cell::Cell;

#[cfg(loom)]
pub use loom::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
#[cfg(not(loom))]
pub use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

#[cfg(loom)]
pub use loom::sync::Arc;
#[cfg(not(loom))]
pub use std::sync::Arc;

pub use antidote::Mutex;
