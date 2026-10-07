//! The popover's HOST card (Nostromo handoff "04 · Host"): CPU, GPU and RAM
//! load with CPU/GPU temperatures, read straight from `/proc` and sysfs at
//! 1 Hz while the popover is open, eased 20 % per tick like the mock. Every
//! reading is optional: a missing source shows "—" and an unlit gauge.

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HostReading {
    pub cpu: Option<f32>,
    pub gpu: Option<f32>,
    pub ram_used: Option<u64>,
    pub ram_total: Option<u64>,
    pub cpu_temp: Option<f32>,
    pub gpu_temp: Option<f32>,
}

/// `(busy, total)` jiffies from `/proc/stat`'s aggregate `cpu` line. Idle
/// and iowait are idle; the first eight fields are the total (guest time
/// is already inside user/nice).
pub fn parse_cpu_times(stat: &str) -> Option<(u64, u64)> {
    let line = stat.lines().find(|l| l.starts_with("cpu "))?;
    let f: Vec<u64> = line.split_whitespace().skip(1).take(8).map(|v| v.parse().ok()).collect::<Option<_>>()?;
    if f.len() < 8 {
        return None;
    }
    let total: u64 = f.iter().sum();
    Some((total - f[3] - f[4], total))
}

pub fn cpu_percent(prev: (u64, u64), now: (u64, u64)) -> Option<f32> {
    let total = now.1.checked_sub(prev.1).filter(|t| *t > 0)?;
    let busy = now.0.checked_sub(prev.0)?;
    Some((busy as f32 / total as f32 * 100.0).clamp(0.0, 100.0))
}

/// `(used, total)` bytes: `MemTotal − MemAvailable`.
pub fn parse_meminfo(s: &str) -> Option<(u64, u64)> {
    let field = |name: &str| {
        s.lines()
            .find(|l| l.starts_with(name))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse::<u64>().ok())
            .map(|kb| kb * 1024)
    };
    let (total, avail) = (field("MemTotal:")?, field("MemAvailable:")?);
    Some((total.saturating_sub(avail), total))
}

pub fn parse_number(s: &str) -> Option<f32> {
    s.trim().parse().ok()
}

/// The mock's smoothing: 20 % of the way to the new reading per tick.
pub fn ease(prev: Option<f32>, target: Option<f32>) -> Option<f32> {
    match (prev, target) {
        (Some(p), Some(t)) => Some(p + (t - p) * 0.2),
        (_, t) => t,
    }
}

/// Where the readings come from, found once at start.
#[derive(Clone, Debug, Default)]
pub struct Sources {
    stat: PathBuf,
    meminfo: PathBuf,
    gpu_busy: Option<PathBuf>,
    cpu_temp: Option<PathBuf>,
    gpu_temp: Option<PathBuf>,
}

/// hwmon drivers whose `temp1_input` is the CPU package temperature.
const CPU_HWMON: [&str; 3] = ["k10temp", "zenpower", "coretemp"];

impl Sources {
    pub fn discover() -> Self {
        Self::discover_in(Path::new("/sys"), Path::new("/proc"))
    }

    pub fn discover_in(sys: &Path, proc_: &Path) -> Self {
        let sorted = |dir: PathBuf| {
            let mut v: Vec<PathBuf> = std::fs::read_dir(dir).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
            v.sort();
            v
        };
        let gpu_busy = sorted(sys.join("class/drm"))
            .into_iter()
            .map(|card| card.join("device/gpu_busy_percent"))
            .find(|p| p.is_file());
        let mut cpu_temp = None;
        let mut gpu_temp = None;
        for hw in sorted(sys.join("class/hwmon")) {
            let name = std::fs::read_to_string(hw.join("name")).unwrap_or_default();
            let input = hw.join("temp1_input");
            if !input.is_file() {
                continue;
            }
            match name.trim() {
                n if CPU_HWMON.contains(&n) && cpu_temp.is_none() => cpu_temp = Some(input),
                "amdgpu" if gpu_temp.is_none() => gpu_temp = Some(input),
                _ => {}
            }
        }
        Sources { stat: proc_.join("stat"), meminfo: proc_.join("meminfo"), gpu_busy, cpu_temp, gpu_temp }
    }
}

pub struct HostSampler {
    sources: Sources,
    prev_cpu: Option<(u64, u64)>,
    eased: HostReading,
}

fn read(p: &Path) -> Option<String> {
    std::fs::read_to_string(p).ok()
}

impl HostSampler {
    pub fn new(sources: Sources) -> Self {
        HostSampler { sources, prev_cpu: None, eased: HostReading::default() }
    }

    /// Drop the CPU baseline and its easing (the popup closed): the next
    /// CPU reading would otherwise average over the closed period. The
    /// first one after this is taken as is, like the very first.
    pub fn forget_cpu(&mut self) {
        self.prev_cpu = None;
        self.eased.cpu = None;
    }

    /// One 1 Hz reading, eased. Blocking file reads of a few bytes each.
    pub fn sample(&mut self) -> HostReading {
        let s = &self.sources;
        let times = read(&s.stat).as_deref().and_then(parse_cpu_times);
        let cpu = match (self.prev_cpu, times) {
            (Some(p), Some(n)) => cpu_percent(p, n),
            _ => None,
        };
        self.prev_cpu = times;
        let gpu = s.gpu_busy.as_deref().and_then(read).as_deref().and_then(parse_number);
        let mem = read(&s.meminfo).as_deref().and_then(parse_meminfo);
        let temp = |p: &Option<PathBuf>| p.as_deref().and_then(read).as_deref().and_then(parse_number).map(|m| m / 1000.0);
        let (cpu_temp, gpu_temp) = (temp(&s.cpu_temp), temp(&s.gpu_temp));
        let e = &mut self.eased;
        e.cpu = ease(e.cpu, cpu);
        e.gpu = ease(e.gpu, gpu);
        e.ram_used = mem.map(|m| m.0);
        e.ram_total = mem.map(|m| m.1);
        e.cpu_temp = cpu_temp;
        e.gpu_temp = gpu_temp;
        *e
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAT: &str = "cpu  100 20 50 800 30 0 10 0 0 0\ncpu0 1 2 3 4 5 6 7 8 0 0\nintr 1\n";

    #[test]
    fn cpu_times_count_iowait_as_idle() {
        // busy = user+nice+system+irq+softirq+steal = 100+20+50+0+10+0
        assert_eq!(parse_cpu_times(STAT), Some((180, 1_010)));
        assert_eq!(parse_cpu_times("intr 1\n"), None);
    }

    #[test]
    fn cpu_percent_is_the_busy_share_of_the_delta() {
        assert_eq!(cpu_percent((180, 1_010), (230, 1_110)), Some(50.0));
        assert_eq!(cpu_percent((180, 1_010), (180, 1_010)), None, "no time passed");
        assert_eq!(cpu_percent((180, 1_010), (100, 900)), None, "counters went backwards");
    }

    #[test]
    fn meminfo_used_is_total_minus_available() {
        let s = "MemTotal:       32654784 kB\nMemFree:  1 kB\nMemAvailable:   21959148 kB\n";
        assert_eq!(parse_meminfo(s), Some(((32_654_784 - 21_959_148) * 1024, 32_654_784 * 1024)));
        assert_eq!(parse_meminfo("MemTotal: 5 kB\n"), None);
    }

    #[test]
    fn numbers_and_easing() {
        assert_eq!(parse_number("40\n"), Some(40.0));
        assert_eq!(parse_number("x"), None);
        assert_eq!(ease(None, Some(50.0)), Some(50.0), "the first reading is taken as is");
        assert_eq!(ease(Some(0.0), Some(50.0)), Some(10.0));
        assert_eq!(ease(Some(10.0), None), None, "a source that vanished reads as missing");
    }

    fn write(p: &std::path::Path, s: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, s).unwrap();
    }

    /// A fake /sys + /proc: amdgpu busy + edge temp, k10temp Tctl.
    #[test]
    fn discovery_and_sampling_on_a_fixture_tree() {
        let root = std::env::temp_dir().join(format!("yutani-host-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (sys, proc_) = (root.join("sys"), root.join("proc"));
        write(&sys.join("class/drm/card1/device/gpu_busy_percent"), "40\n");
        write(&sys.join("class/hwmon/hwmon2/name"), "amdgpu\n");
        write(&sys.join("class/hwmon/hwmon2/temp1_input"), "52000\n");
        write(&sys.join("class/hwmon/hwmon3/name"), "k10temp\n");
        write(&sys.join("class/hwmon/hwmon3/temp1_input"), "46125\n");
        write(&sys.join("class/hwmon/hwmon4/name"), "nvme\n");
        write(&proc_.join("stat"), STAT);
        write(&proc_.join("meminfo"), "MemTotal: 1000 kB\nMemAvailable: 750 kB\n");

        let mut s = HostSampler::new(Sources::discover_in(&sys, &proc_));
        let r = s.sample();
        assert_eq!(r.cpu, None, "CPU needs two samples");
        assert_eq!(r.gpu, Some(40.0));
        assert_eq!((r.ram_used, r.ram_total), (Some(250 * 1024), Some(1000 * 1024)));
        assert_eq!(r.cpu_temp, Some(46.125));
        assert_eq!(r.gpu_temp, Some(52.0));

        write(&proc_.join("stat"), "cpu  150 20 50 850 30 0 10 0 0 0\n");
        let r = s.sample();
        assert_eq!(r.cpu, Some(50.0), "first CPU reading is taken as is");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn forgetting_the_cpu_baseline_restarts_the_reading() {
        let root = std::env::temp_dir().join(format!("yutani-host-forget-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (sys, proc_) = (root.join("sys"), root.join("proc"));
        write(&proc_.join("stat"), STAT);
        let mut s = HostSampler::new(Sources::discover_in(&sys, &proc_));
        s.sample();
        write(&proc_.join("stat"), "cpu  150 20 50 850 30 0 10 0 0 0\n");
        assert_eq!(s.sample().cpu, Some(50.0));
        s.forget_cpu();
        // Much later: a long closed period of mostly idle time.
        write(&proc_.join("stat"), "cpu  160 20 50 9850 30 0 10 0 0 0\n");
        assert_eq!(s.sample().cpu, None, "no baseline: nothing to average over the gap");
        write(&proc_.join("stat"), "cpu  210 20 50 9900 30 0 10 0 0 0\n");
        assert_eq!(s.sample().cpu, Some(50.0), "taken as is, not eased from the old value");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_machine_without_gpu_or_sensors_reads_none() {
        let root = std::env::temp_dir().join(format!("yutani-host-bare-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut s = HostSampler::new(Sources::discover_in(&root.join("sys"), &root.join("proc")));
        assert_eq!(s.sample(), HostReading::default());
    }
}
