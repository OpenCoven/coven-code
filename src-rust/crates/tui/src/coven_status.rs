//! Cached, render-safe views of Coven daemon state.
//!
//! The ratatui draw path runs many times per second; anything it touches must
//! be non-blocking. These helpers wrap the blocking probes in
//! [`claurst_core::coven_shared`] behind short-TTL caches so the welcome
//! panel and familiar switcher never do socket or disk I/O per frame.
//!
//! Render-only: security-critical agent-merge paths (tool filtering, agent
//! resolution) must keep calling `coven_shared::load_familiars()` directly so
//! a roster edit is always honored at the moment a security decision is made.

use claurst_core::coven_shared::{self, CovenFamiliar, DaemonClient};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Daemon reachability (welcome panel)
// ---------------------------------------------------------------------------

/// How long a completed reachability probe stays fresh.
const DAEMON_PROBE_TTL: Duration = Duration::from_secs(10);

/// Budget for one background probe. Long enough that a busy daemon doesn't
/// flip the welcome panel to "offline" mid-load — see issue #50.
const DAEMON_PROBE_BUDGET: Duration = Duration::from_millis(2000);

struct DaemonProbe {
    checked_at: Instant,
    online: bool,
}

fn daemon_probe_cell() -> &'static Mutex<Option<DaemonProbe>> {
    static CELL: OnceLock<Mutex<Option<DaemonProbe>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(None))
}

static DAEMON_PROBE_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// Whether the Coven daemon looks online, without ever blocking the caller.
///
/// Returns the last cached reachability result and kicks off a background
/// refresh when the cache is older than [`DAEMON_PROBE_TTL`]. Until the first
/// probe completes, falls back to socket-file presence, which is instant and
/// almost always right; the real probe result lands a frame or two later.
pub fn daemon_looks_online() -> bool {
    let now = Instant::now();
    let cached = {
        let guard = daemon_probe_cell()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        guard
            .as_ref()
            .map(|p| (p.online, now.duration_since(p.checked_at)))
    };

    let stale = match cached {
        Some((_, age)) => age >= DAEMON_PROBE_TTL,
        None => true,
    };
    if stale
        && DAEMON_PROBE_IN_FLIGHT
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    {
        // Resolve the socket path on the caller thread (it reads env vars,
        // which tests mutate under a lock); only the blocking probe itself
        // runs on the background thread.
        let client = DaemonClient::new();
        std::thread::spawn(move || {
            let online = client
                .map(|c| c.check_reachability(DAEMON_PROBE_BUDGET).looks_alive())
                .unwrap_or(false);
            let mut guard = daemon_probe_cell()
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            *guard = Some(DaemonProbe {
                checked_at: Instant::now(),
                online,
            });
            DAEMON_PROBE_IN_FLIGHT.store(false, Ordering::Release);
        });
    }

    match cached {
        Some((online, _)) => online,
        None => coven_shared::daemon_socket_present(),
    }
}

// ---------------------------------------------------------------------------
// Familiar roster (~/.coven/familiars.toml)
// ---------------------------------------------------------------------------

/// How long a loaded roster stays fresh for render purposes. Short enough
/// that an external edit shows up promptly, long enough that a draw loop
/// never re-reads the file more than once per interval.
const ROSTER_TTL: Duration = Duration::from_secs(2);

type RosterCache = Option<(Instant, Vec<CovenFamiliar>)>;

fn roster_cache() -> &'static Mutex<RosterCache> {
    static CACHE: OnceLock<Mutex<RosterCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// TTL-cached view of the familiar roster for render paths.
///
/// Missing/malformed/empty roster all collapse to an empty vec, mirroring how
/// the render call-sites already treated `load_familiars().unwrap_or_default()`.
pub fn cached_familiars() -> Vec<CovenFamiliar> {
    let now = Instant::now();
    {
        let guard = roster_cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some((loaded_at, fams)) = &*guard {
            if now.duration_since(*loaded_at) < ROSTER_TTL {
                return fams.clone();
            }
        }
    }
    let fams = coven_shared::load_familiars().unwrap_or_default();
    let mut guard = roster_cache().lock().unwrap_or_else(|e| e.into_inner());
    *guard = Some((now, fams.clone()));
    fams
}

/// Drop the cached roster so the next render re-reads the file. Call after
/// any in-process write to `~/.coven/familiars.toml` (starter bootstrap,
/// editor remove, roster reset) so the UI reflects the change immediately.
pub fn invalidate_roster_cache() {
    let mut guard = roster_cache().lock().unwrap_or_else(|e| e.into_inner());
    *guard = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalidate_clears_cached_roster() {
        {
            let mut guard = roster_cache().lock().unwrap_or_else(|e| e.into_inner());
            *guard = Some((Instant::now(), Vec::new()));
        }
        invalidate_roster_cache();
        let guard = roster_cache().lock().unwrap_or_else(|e| e.into_inner());
        assert!(guard.is_none());
    }

    #[test]
    fn daemon_looks_online_returns_quickly() {
        // The whole point of this helper: it must not block on a socket
        // round-trip. Allow generous slack for CI schedulers while still
        // catching a 2 s synchronous probe.
        let start = Instant::now();
        let _ = daemon_looks_online();
        assert!(
            start.elapsed() < Duration::from_millis(500),
            "daemon_looks_online blocked the caller for {:?}",
            start.elapsed()
        );
    }
}
