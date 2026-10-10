//! Resident-set-size reader over `/proc/<pid>/status` (Linux).
//!
//! Promoted from `pglet_bench`'s `memory` scenario so the soak harness can sample
//! a **spawned server child's** peak/current RSS by pid (the bench only ever needs
//! its own process, `pid = None`). `pglet_bench` keeps its own copy: it is a
//! production binary in the `check:release` audit and must not gain a dev-only
//! `pglet_testing` edge — the duplication is deliberate and audit-required.
//!
//! Linux-only (reads `/proc`); a non-Linux run returns `None` so the caller can
//! degrade to 0 rather than abort.

/// Read a `/proc/<pid>/status` numeric kilobyte field (e.g. `VmHWM`, `VmRSS`).
///
/// `pid = None` reads `/proc/self/status` (this process); `Some(p)` reads
/// `/proc/<p>/status` (a spawned child). Returns `None` off Linux, if the process
/// is gone, or if the field is absent.
#[must_use]
pub fn read_status_kb(pid: Option<u32>, field: &str) -> Option<u64> {
    let path = pid.map_or_else(
        || "/proc/self/status".to_string(),
        |p| format!("/proc/{p}/status"),
    );
    let status = std::fs::read_to_string(path).ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix(field) {
            // Line shape: "VmHWM:\t   12345 kB" — strip the colon, take the number.
            let rest = rest.trim_start_matches(':').trim();
            let kb = rest.split_whitespace().next()?;
            return kb.parse().ok();
        }
    }
    None
}
