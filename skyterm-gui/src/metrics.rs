//! Live system metrics (load average, CPU, memory, swap) for the title bar.
//!
//! Linux only: everything is read from `/proc`, so there is no extra
//! dependency and a refresh costs microseconds. On other platforms
//! [`Sampler::sample`] returns `None` and the title bar shows nothing.
//!
//! The parsers take the file *contents* as `&str` so they can be unit-tested
//! against fixed text; only [`Sampler::sample`] touches the filesystem.

/// Seconds between refreshes unless configured otherwise.
pub const DEFAULT_INTERVAL_SECS: u32 = 5;
pub const MIN_INTERVAL_SECS: u32 = 1;
pub const MAX_INTERVAL_SECS: u32 = 60;

/// Clamp a configured interval into the supported range.
pub fn clamp_interval(secs: u32) -> u32 {
    secs.clamp(MIN_INTERVAL_SECS, MAX_INTERVAL_SECS)
}

/// Whether this platform can report metrics at all.
pub const SUPPORTED: bool = cfg!(target_os = "linux");

/// Cumulative CPU jiffies from the aggregate `cpu` line of `/proc/stat`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuTimes {
    pub idle: u64,
    pub total: u64,
}

/// One reading of everything shown in the title bar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    /// Busy share of all CPUs since the previous sample, 0–100. `None` on the
    /// very first sample (a percentage needs two readings).
    pub cpu: Option<f32>,
    pub mem: f32,
    pub swap: f32,
    pub load: [f32; 3],
}

/// Parse the aggregate `cpu ` line of `/proc/stat`.
pub fn parse_cpu_times(stat: &str) -> Option<CpuTimes> {
    let line = stat.lines().find(|l| l.starts_with("cpu "))?;
    let f: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .take(8) // user nice system idle iowait irq softirq steal
        .map(|v| v.parse().ok())
        .collect::<Option<_>>()?;
    if f.len() < 4 {
        return None;
    }
    let idle = f[3] + f.get(4).copied().unwrap_or(0); // idle + iowait
    Some(CpuTimes { idle, total: f.iter().sum() })
}

/// CPU busy percentage between two readings.
pub fn cpu_percent(prev: CpuTimes, cur: CpuTimes) -> Option<f32> {
    let total = cur.total.checked_sub(prev.total)?;
    let idle = cur.idle.checked_sub(prev.idle)?;
    if total == 0 {
        return None;
    }
    Some(((total - idle.min(total)) as f32 / total as f32 * 100.0).clamp(0.0, 100.0))
}

fn meminfo_kb(meminfo: &str, key: &str) -> Option<u64> {
    meminfo
        .lines()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// `(mem_used_percent, swap_used_percent)` from `/proc/meminfo`. Memory uses
/// `MemAvailable` (what `free` and `top` report), so page cache doesn't count
/// as used. A machine with no swap reports 0% swap.
pub fn parse_meminfo(meminfo: &str) -> Option<(f32, f32)> {
    let total = meminfo_kb(meminfo, "MemTotal")?;
    let avail = meminfo_kb(meminfo, "MemAvailable")?;
    if total == 0 {
        return None;
    }
    let mem = (total.saturating_sub(avail)) as f32 / total as f32 * 100.0;
    let swap = match (meminfo_kb(meminfo, "SwapTotal"), meminfo_kb(meminfo, "SwapFree")) {
        (Some(t), Some(f)) if t > 0 => t.saturating_sub(f) as f32 / t as f32 * 100.0,
        _ => 0.0,
    };
    Some((mem.clamp(0.0, 100.0), swap.clamp(0.0, 100.0)))
}

/// The 1/5/15-minute load averages from `/proc/loadavg`.
pub fn parse_loadavg(loadavg: &str) -> Option<[f32; 3]> {
    let mut it = loadavg.split_whitespace().map(|v| v.parse::<f32>().ok());
    Some([it.next()??, it.next()??, it.next()??])
}

impl Metrics {
    /// The title-bar line, e.g. `load avg: [0.5, 1.2, 2.0]  cpu:30%  mem:50%  swap:20%`.
    pub fn summary(&self) -> String {
        let cpu = self.cpu.map_or("--".to_string(), |c| format!("{c:.0}"));
        format!(
            "load avg: [{:.1}, {:.1}, {:.1}]  cpu:{cpu}%  mem:{:.0}%  swap:{:.0}%",
            self.load[0], self.load[1], self.load[2], self.mem, self.swap
        )
    }

    /// Longer multi-line form for the tooltip.
    pub fn details(&self) -> String {
        let cpu = self.cpu.map_or("measuring…".to_string(), |c| format!("{c:.1}%"));
        format!(
            "Load average (1/5/15 min): {:.2}  {:.2}  {:.2}\nCPU: {cpu}\nMemory used: {:.1}%\nSwap used: {:.1}%",
            self.load[0], self.load[1], self.load[2], self.mem, self.swap
        )
    }
}

/// Reads the metrics, remembering the previous CPU reading so it can report a
/// percentage.
#[derive(Default)]
pub struct Sampler {
    prev_cpu: Option<CpuTimes>,
}

impl Sampler {
    /// Forget the previous CPU reading (e.g. after the interval changed, so
    /// the next percentage isn't averaged over a stale gap).
    pub fn reset(&mut self) {
        self.prev_cpu = None;
    }

    /// Take a reading. `None` when unsupported or `/proc` is unreadable.
    #[cfg(target_os = "linux")]
    pub fn sample(&mut self) -> Option<Metrics> {
        use std::fs::read_to_string;
        let cur = parse_cpu_times(&read_to_string("/proc/stat").ok()?)?;
        let cpu = self.prev_cpu.and_then(|p| cpu_percent(p, cur));
        self.prev_cpu = Some(cur);
        let (mem, swap) = parse_meminfo(&read_to_string("/proc/meminfo").ok()?)?;
        let load = parse_loadavg(&read_to_string("/proc/loadavg").ok()?)?;
        Some(Metrics { cpu, mem, swap, load })
    }

    #[cfg(not(target_os = "linux"))]
    pub fn sample(&mut self) -> Option<Metrics> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAT: &str = "cpu  4705 150 1120 16250 520 30 45 10 0 0\n\
                        cpu0 1200 40 300 4000 100 10 20 2 0 0\n\
                        intr 12345\n";

    #[test]
    fn parses_aggregate_cpu_line_not_per_core() {
        let t = parse_cpu_times(STAT).unwrap();
        // idle = idle(16250) + iowait(520); total = first 8 fields.
        assert_eq!(t.idle, 16770);
        assert_eq!(t.total, 4705 + 150 + 1120 + 16250 + 520 + 30 + 45 + 10);
    }

    #[test]
    fn cpu_stat_garbage_is_rejected() {
        assert_eq!(parse_cpu_times(""), None);
        assert_eq!(parse_cpu_times("cpu0 1 2 3 4\n"), None, "per-core line only");
        assert_eq!(parse_cpu_times("cpu  1 2 x 4\n"), None);
        assert_eq!(parse_cpu_times("cpu  1 2\n"), None, "too few fields");
    }

    #[test]
    fn cpu_percent_between_samples() {
        let a = CpuTimes { idle: 800, total: 1000 };
        let b = CpuTimes { idle: 850, total: 1100 }; // 100 jiffies, 50 idle
        assert_eq!(cpu_percent(a, b), Some(50.0));
        let all_busy = CpuTimes { idle: 800, total: 1100 };
        assert_eq!(cpu_percent(a, all_busy), Some(100.0));
        let all_idle = CpuTimes { idle: 900, total: 1100 };
        assert_eq!(cpu_percent(a, all_idle), Some(0.0));
    }

    #[test]
    fn cpu_percent_needs_elapsed_time_and_monotonic_counters() {
        let a = CpuTimes { idle: 10, total: 100 };
        assert_eq!(cpu_percent(a, a), None, "no time elapsed");
        let earlier = CpuTimes { idle: 5, total: 50 };
        assert_eq!(cpu_percent(a, earlier), None, "counters went backwards");
    }

    const MEMINFO: &str = "MemTotal:       16000000 kB\n\
                           MemFree:         1000000 kB\n\
                           MemAvailable:    8000000 kB\n\
                           Buffers:          500000 kB\n\
                           SwapTotal:       4000000 kB\n\
                           SwapFree:        3200000 kB\n";

    #[test]
    fn memory_uses_available_not_free() {
        let (mem, swap) = parse_meminfo(MEMINFO).unwrap();
        assert!((mem - 50.0).abs() < 0.01, "mem {mem}");
        assert!((swap - 20.0).abs() < 0.01, "swap {swap}");
    }

    #[test]
    fn no_swap_is_zero_percent() {
        let m = "MemTotal: 1000 kB\nMemAvailable: 250 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB\n";
        let (mem, swap) = parse_meminfo(m).unwrap();
        assert!((mem - 75.0).abs() < 0.01);
        assert_eq!(swap, 0.0);
        // Missing swap lines entirely is also fine.
        let (_, swap) = parse_meminfo("MemTotal: 1000 kB\nMemAvailable: 250 kB\n").unwrap();
        assert_eq!(swap, 0.0);
    }

    #[test]
    fn meminfo_without_totals_is_rejected() {
        assert_eq!(parse_meminfo(""), None);
        assert_eq!(parse_meminfo("MemTotal: 0 kB\nMemAvailable: 0 kB\n"), None);
        assert_eq!(parse_meminfo("MemTotal: 100 kB\n"), None, "no MemAvailable");
    }

    #[test]
    fn loadavg_parses_three_values() {
        assert_eq!(parse_loadavg("0.52 1.25 2.01 1/456 7890\n"), Some([0.52, 1.25, 2.01]));
        assert_eq!(parse_loadavg("0.52 1.25\n"), None);
        assert_eq!(parse_loadavg("a b c\n"), None);
    }

    #[test]
    fn summary_format() {
        let m = Metrics { cpu: Some(30.4), mem: 50.0, swap: 20.0, load: [0.52, 1.25, 2.01] };
        assert_eq!(m.summary(), "load avg: [0.5, 1.2, 2.0]  cpu:30%  mem:50%  swap:20%");
        let first = Metrics { cpu: None, ..m };
        assert!(first.summary().contains("cpu:--%"), "{}", first.summary());
        assert!(m.details().contains("CPU: 30.4%"));
        assert!(first.details().contains("measuring"));
    }

    #[test]
    fn interval_is_clamped() {
        assert_eq!(DEFAULT_INTERVAL_SECS, 5);
        assert_eq!(clamp_interval(0), MIN_INTERVAL_SECS);
        assert_eq!(clamp_interval(5), 5);
        assert_eq!(clamp_interval(10_000), MAX_INTERVAL_SECS);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn real_proc_sampling_works() {
        let mut s = Sampler::default();
        let first = s.sample().expect("/proc readable on Linux");
        assert_eq!(first.cpu, None, "no CPU percentage until the second sample");
        assert!((0.0..=100.0).contains(&first.mem) && (0.0..=100.0).contains(&first.swap));
        assert!(first.load.iter().all(|l| *l >= 0.0));
        // Burn a little CPU so two samples differ in time.
        let t = std::time::Instant::now();
        while t.elapsed() < std::time::Duration::from_millis(60) {
            std::hint::black_box(0u64);
        }
        let second = s.sample().unwrap();
        if let Some(c) = second.cpu {
            assert!((0.0..=100.0).contains(&c));
        }
        s.reset();
        assert_eq!(s.sample().unwrap().cpu, None, "reset forgets the baseline");
    }
}
