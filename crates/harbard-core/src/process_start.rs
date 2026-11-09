//! Process start markers: the second half of a PID identity.
//!
//! A PID alone cannot prove a process is the one we saw earlier — the kernel
//! recycles them. Pairing the PID with the moment the process started makes
//! the pair unique for the life of the machine (modulo a reboot that hands the
//! same PID to a process started at the same tick, which a long-lived harness
//! never is).
//!
//! The value is opaque and platform-specific: compare it for equality only,
//! never as a timestamp.
//! - macOS: `proc_pidinfo(PROC_PIDTBSDINFO)` start, as microseconds since the
//!   Unix epoch.
//! - Linux: `/proc/<pid>/stat` field 22, clock ticks since boot.
//! - Anything else: `None`, so callers fall back to time-based presence.

/// Start marker of `pid`, or `None` when the process does not exist, cannot be
/// inspected, or the platform is unsupported.
pub fn process_start_time(pid: u32) -> Option<u64> {
    imp::start_time(pid)
}

#[cfg(target_os = "macos")]
mod imp {
    pub fn start_time(pid: u32) -> Option<u64> {
        let raw = libc::c_int::try_from(pid).ok()?;
        if raw <= 0 {
            return None;
        }
        let size = std::mem::size_of::<libc::proc_bsdinfo>();
        let size_c = libc::c_int::try_from(size).ok()?;
        let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
        // SAFETY: `info` is a properly sized, writable buffer for exactly one
        // `proc_bsdinfo`, and its size is passed so the kernel never writes
        // past it. The return value is checked before the buffer is read.
        let written = unsafe {
            libc::proc_pidinfo(
                raw,
                libc::PROC_PIDTBSDINFO,
                0,
                info.as_mut_ptr().cast::<libc::c_void>(),
                size_c,
            )
        };
        if written != size_c {
            return None;
        }
        // SAFETY: the kernel filled the whole struct (checked above), and the
        // zeroed initialisation already made every field a valid integer.
        let info = unsafe { info.assume_init() };
        info.pbi_start_tvsec
            .checked_mul(1_000_000)?
            .checked_add(info.pbi_start_tvusec)
    }
}

#[cfg(target_os = "linux")]
mod imp {
    pub fn start_time(pid: u32) -> Option<u64> {
        if pid == 0 {
            return None;
        }
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        super::parse_linux_stat_start(&stat)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod imp {
    pub fn start_time(_pid: u32) -> Option<u64> {
        None
    }
}

/// Field 22 (`starttime`) of a `/proc/<pid>/stat` line. The command name in
/// field 2 is parenthesised and may itself contain spaces or `)`, so fields are
/// counted from the LAST `)`: what follows it starts at field 3.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_linux_stat_start(stat: &str) -> Option<u64> {
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(22 - 3)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_stat_start_skips_a_hostile_command_name() {
        let line = "4242 (evil) name) S 1 4242 4242 0 -1 4194560 100 0 0 0 \
                    5 3 0 0 20 0 1 0 987654 12345678 300 18446744073709551615";
        assert_eq!(parse_linux_stat_start(line), Some(987_654));
        assert_eq!(parse_linux_stat_start("4242 (x) S 1"), None);
        assert_eq!(parse_linux_stat_start("no parens"), None);
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn own_process_has_a_stable_start_marker() {
        let me = std::process::id();
        let first = process_start_time(me).expect("own start time");
        assert_eq!(process_start_time(me), Some(first));
        assert_eq!(process_start_time(0), None);
    }
}
