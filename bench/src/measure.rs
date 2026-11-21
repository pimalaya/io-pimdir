//! What a phase costs: wall time, CPU time and the bytes it moved, read
//! from the process itself so that a run prints one line a script can parse.

use std::{fs, mem::MaybeUninit, time::Instant};

/// The counters of `/proc/self/io` a phase reports.
#[derive(Clone, Copy, Default)]
struct ProcIo {
    rchar: u64,
    wchar: u64,
    read_bytes: u64,
    write_bytes: u64,
    cancelled_write_bytes: u64,
}

impl ProcIo {
    fn read() -> Self {
        let mut io = Self::default();
        let text = fs::read_to_string("/proc/self/io").unwrap_or_default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let value = value.trim().parse().unwrap_or(0);
            match key {
                "rchar" => io.rchar = value,
                "wchar" => io.wchar = value,
                "read_bytes" => io.read_bytes = value,
                "write_bytes" => io.write_bytes = value,
                "cancelled_write_bytes" => io.cancelled_write_bytes = value,
                _ => {}
            }
        }
        io
    }
}

/// User and system CPU time of the process so far, in microseconds.
fn cpu_micros() -> (u64, u64) {
    // SAFETY: getrusage fills a zeroed plain-data struct, RUSAGE_SELF is valid
    let usage = unsafe {
        let mut usage = MaybeUninit::<libc::rusage>::zeroed();
        libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr());
        usage.assume_init()
    };
    let micros = |t: libc::timeval| t.tv_sec as u64 * 1_000_000 + t.tv_usec as u64;
    (micros(usage.ru_utime), micros(usage.ru_stime))
}

/// The peak resident set of the process so far, in KiB.
pub fn peak_rss_kib() -> u64 {
    fs::read_to_string("/proc/self/status")
        .unwrap_or_default()
        .lines()
        .find_map(|l| l.strip_prefix("VmHWM:"))
        .and_then(|v| v.trim().trim_end_matches(" kB").trim().parse().ok())
        .unwrap_or(0)
}

/// The counters at the start of a phase.
pub struct Phase {
    at: Instant,
    user: u64,
    sys: u64,
    io: ProcIo,
}

impl Phase {
    pub fn start() -> Self {
        let (user, sys) = cpu_micros();
        Self {
            at: Instant::now(),
            user,
            sys,
            io: ProcIo::read(),
        }
    }

    /// The seconds since the start, as a timer would read them.
    pub fn wall_secs(&self) -> f64 {
        self.at.elapsed().as_secs_f64()
    }

    /// One `key=value` line for the phase since its start, `extra` pairs
    /// appended in their order.
    pub fn line(&self, phase: &str, items: u64, extra: &[(&str, String)]) -> String {
        let wall = self.wall_secs();
        let (user, sys) = cpu_micros();
        let io = ProcIo::read();
        let mut line = format!(
            "phase={phase} wall_s={wall:.3} user_s={:.3} sys_s={:.3} write_bytes={} cancelled_write_bytes={} wchar={} read_bytes={} rchar={} items={items} peak_rss_kib={}",
            (user - self.user) as f64 / 1e6,
            (sys - self.sys) as f64 / 1e6,
            io.write_bytes - self.io.write_bytes,
            io.cancelled_write_bytes - self.io.cancelled_write_bytes,
            io.wchar - self.io.wchar,
            io.read_bytes - self.io.read_bytes,
            io.rchar - self.io.rchar,
            peak_rss_kib(),
        );
        for (key, value) in extra {
            line.push(' ');
            line.push_str(key);
            line.push('=');
            line.push_str(value);
        }
        line
    }
}
