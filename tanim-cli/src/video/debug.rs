#![cfg(feature = "debug")]

use once_cell::sync::Lazy;
use std::{
    collections::HashMap,
    sync::{
        LockResult, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread::{self, ThreadId},
};
use tracing::info;

/// A unique identifier for a lock.
pub type LockId = usize;

/// The state of a lock.
#[derive(Debug, Clone)]
pub enum LockState {
    /// The lock is currently held by a thread.
    Held,
    /// A thread is waiting to acquire the lock.
    Waiting,
}

/// The state of a thread.
#[derive(Debug, Clone, Default)]
pub struct ThreadState {
    /// The name of the thread.
    pub name: String,
    /// The locks that the thread is currently interacting with.
    pub locks: HashMap<LockId, LockState>,
    /// Some data associated with the thread.
    pub data: String,
    /// The current status of the thread.
    pub status: String,
}

/// Sets the status of the current thread.
pub fn set_thread_status(status: String) {
    let thread_id = thread::current().id();
    if let Some(state) = GLOBAL_STATE.lock().unwrap().get_mut(&thread_id) {
        state.status = status;
    }
}

/// Information about a lock.
#[derive(Debug, Clone)]
pub struct LockInfo {
    /// The name of the lock.
    pub name: String,
}

/// The global state of all threads and locks.
static GLOBAL_STATE: Lazy<Mutex<HashMap<ThreadId, ThreadState>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static LOCK_INFO: Lazy<Mutex<HashMap<LockId, LockInfo>>> = Lazy::new(|| Mutex::new(HashMap::new()));

/// A counter for generating unique lock IDs.
static NEXT_LOCK_ID: AtomicUsize = AtomicUsize::new(0);

/// Generates a new unique lock ID.
pub fn new_lock_id(name: &str) -> LockId {
    let id = NEXT_LOCK_ID.fetch_add(1, Ordering::SeqCst);
    LOCK_INFO.lock().unwrap().insert(
        id,
        LockInfo {
            name: name.to_string(),
        },
    );
    id
}
/// A wrapper around `std::sync::Mutex` that tracks its state.
#[derive(Debug)]
pub struct DebugMutex<T> {
    id: LockId,
    name: String,
    mutex: Mutex<T>,
}

impl<T> DebugMutex<T> {
    /// Creates a new `DebugMutex`.
    pub fn new(value: T, name: &str) -> Self {
        Self {
            id: new_lock_id(name),
            name: name.to_string(),
            mutex: Mutex::new(value),
        }
    }

    /// Acquires the lock, blocking the current thread until it is able to do so.
    pub fn lock(
        &self,
    ) -> Result<DebugMutexGuard<'_, T>, std::sync::PoisonError<std::sync::MutexGuard<'_, T>>> {
        let thread_id = thread::current().id();

        // Mark the thread as waiting for the lock.
        GLOBAL_STATE
            .lock()
            .unwrap()
            .entry(thread_id)
            .or_default()
            .locks
            .insert(self.id, LockState::Waiting);

        let guard = self.mutex.lock();

        // Mark the lock as held by the thread.
        GLOBAL_STATE
            .lock()
            .unwrap()
            .entry(thread_id)
            .or_default()
            .locks
            .insert(self.id, LockState::Held);

        guard.map(|g| DebugMutexGuard {
            mutex: self,
            guard: g,
        })
    }
}

/// A RAII implementation of a scoped lock for a `DebugMutex`.
/// When this structure is dropped (falls out of scope), the lock will be unlocked.
pub struct DebugMutexGuard<'a, T: 'a> {
    mutex: &'a DebugMutex<T>,
    guard: std::sync::MutexGuard<'a, T>,
}

impl<'a, T> std::ops::Deref for DebugMutexGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.guard.deref()
    }
}

impl<'a, T> std::ops::DerefMut for DebugMutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard.deref_mut()
    }
}

impl<'a, T> Drop for DebugMutexGuard<'a, T> {
    fn drop(&mut self) {
        let thread_id = thread::current().id();
        if let Some(state) = GLOBAL_STATE.lock().unwrap().get_mut(&thread_id) {
            state.locks.remove(&self.mutex.id);
        }
        // The lock is automatically released when the inner guard is dropped.
    }
}
/// A RAII guard that registers the current thread in the global state.
pub struct ThreadTracker {
    thread_id: ThreadId,
}

impl ThreadTracker {
    /// Creates a new `ThreadTracker` and registers the current thread.
    pub fn new(name: String) -> Self {
        let thread_id = thread::current().id();
        let mut state = GLOBAL_STATE.lock().unwrap();
        state.entry(thread_id).or_insert_with(|| ThreadState {
            name,
            ..Default::default()
        });
        Self { thread_id }
    }
}

impl Drop for ThreadTracker {
    fn drop(&mut self) {
        GLOBAL_STATE.lock().unwrap().remove(&self.thread_id);
    }
}
/// Dumps the state of all threads and locks to the log.
pub fn dump_deadlock_info() {
    info!("--- Deadlock Info Dump ---");
    let state = GLOBAL_STATE.lock().unwrap();
    let lock_info = LOCK_INFO.lock().unwrap();
    for (thread_id, thread_state) in state.iter() {
        info!("Thread ID: {:?}, Name: {}", thread_id, thread_state.name);
        if !thread_state.locks.is_empty() {
            for (lock_id, lock_state) in thread_state.locks.iter() {
                let state_str = match lock_state {
                    LockState::Held => "Held",
                    LockState::Waiting => "Waiting",
                };
                let lock_name = lock_info
                    .get(lock_id)
                    .map(|info| info.name.as_str())
                    .unwrap_or("Unknown");
                info!(
                    "  - Lock ID: {}, Name: {}, State: {}",
                    lock_id, lock_name, state_str
                );
            }
        } else {
            info!("  - No locks held or waiting.");
        }
        if !thread_state.data.is_empty() {
            info!("  - Data: {}", thread_state.data);
        }
        if !thread_state.status.is_empty() {
            info!("  - Status: {}", thread_state.status);
        }
    }
    info!("--- End of Dump ---");
}
