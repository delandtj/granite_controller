//! The memory budget (ADR 0001 component 27).
//!
//! The ESP32-C6 has no external RAM and a Rust allocation that fails
//! aborts the firmware, so this module holds the two mechanisms that
//! keep the margin: the heap figures everything else reports, and the
//! one token the heavy users take turns on.
//!
//! ### The figures
//!
//! [`free`], [`minimum_free`] and [`largest_free_block`] are the three
//! numbers `status` carries and the heartbeat samples: how much is
//! left, how little there has ever been since boot, and how big a
//! single allocation can still be. The last one is the one that
//! actually fails a request on a fragmented heap.
//!
//! ### The gate
//!
//! [`heavy`] hands out one token. Whoever is about to use tens of
//! kilobytes for a while takes it first and holds it until the guard
//! drops:
//!
//! - an OTA pull for its whole call, TLS handshake included
//!   (`ota::pull`, [`PULL_WAIT`]),
//! - a firmware upload while its slot is open (`OtaWriter::begin` to
//!   `finish` or `abort`, [`UPLOAD_WAIT`]); an upload that cannot have
//!   the token is refused as busy,
//! - one switch request, from the connect to the end of its parse
//!   (`switch::client`, its request budget).
//!
//! It replaces the special case of ADR 0006 component 17 (the switch
//! worker standing still while a firmware image comes in): the worker
//! simply does not get the token, and the request it was about to make
//! fails as busy.
//!
//! The token is **not** re-entrant. One task may hold it once; a
//! second take from the same thread would wait for itself, so it is
//! refused at once with an error in the log instead. That is why a
//! pull hands its guard to the writer it opens
//! ([`crate::platform::ota::OtaWriter::hold_heavy`]) rather than
//! letting the writer take a second one.
//!
//! A taker waits with a timeout and feeds the task watchdog while it
//! waits, because the threads that take it (the switch worker, the
//! dispatcher) are under it.

use std::sync::Mutex;
use std::thread;
use std::thread::ThreadId;
use std::time::Duration;

/// How long an OTA pull waits for the token before it gives up.
pub const PULL_WAIT: Duration = Duration::from_secs(20);

/// How long a firmware upload waits before it is refused as busy. The
/// HTTP client is sitting on an open request, so this is short.
pub const UPLOAD_WAIT: Duration = Duration::from_secs(5);

/// How often a waiting taker looks again.
const POLL: Duration = Duration::from_millis(50);

/// A wait longer than this is worth a log line.
const NOISY_WAIT_MS: u64 = 1_000;

/// Free heap in bytes.
pub fn free() -> u32 {
    unsafe { esp_idf_svc::sys::esp_get_free_heap_size() }
}

/// Least free heap since boot, in bytes.
pub fn minimum_free() -> u32 {
    unsafe { esp_idf_svc::sys::esp_get_minimum_free_heap_size() }
}

/// Largest block a single allocation could still get, in bytes.
pub fn largest_free_block() -> u32 {
    let n = unsafe {
        esp_idf_svc::sys::heap_caps_get_largest_free_block(esp_idf_svc::sys::MALLOC_CAP_DEFAULT)
    };
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The three figures `status` publishes: free, least free since boot,
/// largest free block.
pub fn figures() -> (u32, u32, u32) {
    (free(), minimum_free(), largest_free_block())
}

/// One line about the heap, for a log message.
pub fn summary() -> String {
    let (free, low, largest) = figures();
    format!("free {free} bytes, minimum {low} bytes, largest free block {largest} bytes")
}

/// Who holds the token.
struct Holder {
    who: &'static str,
    thread: ThreadId,
    since_ms: u64,
}

/// The token itself: `None` means free. A plain mutex around an option
/// rather than a FreeRTOS semaphore, so the holder can be named in a
/// log line and a re-entrant take can be refused instead of hanging.
static GATE: Mutex<Option<Holder>> = Mutex::new(None);

/// The token, held until this is dropped.
///
/// Not `Clone`, not `Send`-restricted: a pull hands it to the
/// [`crate::platform::ota::OtaWriter`] it opens, which lives on the
/// same thread but is stored in a struct.
#[derive(Debug)]
pub struct Heavy {
    who: &'static str,
}

impl Heavy {
    /// What this guard was taken for.
    pub const fn who(&self) -> &'static str {
        self.who
    }
}

impl Drop for Heavy {
    fn drop(&mut self) {
        let mut gate = lock();
        let held_ms = gate
            .as_ref()
            .map(|h| crate::platform::now_ms().saturating_sub(h.since_ms));
        *gate = None;
        drop(gate);
        if let Some(ms) = held_ms
            && ms > NOISY_WAIT_MS
        {
            log::debug!("mem: {} released the heavy token after {ms} ms", self.who);
        }
    }
}

/// The gate's lock, recovering from a poisoned mutex: a panic while the
/// token was held must not wedge the token for good.
fn lock() -> std::sync::MutexGuard<'static, Option<Holder>> {
    GATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Take the one heavy-memory token, waiting at most `wait`.
///
/// `None` means busy: the caller reports that, it does not go ahead
/// anyway. `who` is a fixed name for the log ("ota pull", "switch
/// request", "firmware upload").
pub fn heavy(who: &'static str, wait: Duration) -> Option<Heavy> {
    let me = thread::current().id();
    let started = crate::platform::now_ms();
    let deadline = started + wait.as_millis() as u64;
    loop {
        let holder = {
            let mut gate = lock();
            match gate.as_ref() {
                None => {
                    *gate = Some(Holder {
                        who,
                        thread: me,
                        since_ms: crate::platform::now_ms(),
                    });
                    drop(gate);
                    let waited = crate::platform::now_ms().saturating_sub(started);
                    if waited > NOISY_WAIT_MS {
                        log::info!("mem: {who} waited {waited} ms for the heavy token");
                    }
                    return Some(Heavy { who });
                }
                Some(held) if held.thread == me => {
                    // Waiting here would be waiting for ourselves.
                    log::error!(
                        "mem: {who} asked for the heavy token this task already holds for {}; \
                         refusing instead of deadlocking (a bug in the caller)",
                        held.who
                    );
                    return None;
                }
                Some(held) => held.who,
            }
        };
        if crate::platform::now_ms() >= deadline {
            log::warn!(
                "mem: {who} did not get the heavy token within {} ms ({holder} has it); busy",
                wait.as_millis()
            );
            return None;
        }
        // The takers (the switch worker, the dispatcher) are under the
        // task watchdog and this wait can be seconds long.
        crate::platform::feed_task_watchdog();
        thread::sleep(POLL);
    }
}

/// What the token is being held for right now, for a log line or the
/// console. Empty when it is free.
pub fn heavy_holder() -> &'static str {
    lock().as_ref().map_or("", |h| h.who)
}
