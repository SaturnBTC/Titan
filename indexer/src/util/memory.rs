use std::{fs, time::{Duration, Instant}};

#[derive(Debug, Clone, Copy)]
pub struct MemorySnapshot {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub process_rss_bytes: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct MemoryLimiterSettings {
    /// Trigger when used >= total * high_watermark_ratio
    pub high_watermark_ratio: f64,
    /// Consider recovered when used <= total * low_watermark_ratio
    pub low_watermark_ratio: f64,
    /// Minimum interval between expensive reads
    pub min_refresh_interval: Duration,
}

impl Default for MemoryLimiterSettings {
    fn default() -> Self {
        Self {
            high_watermark_ratio: 0.85,
            low_watermark_ratio: 0.75,
            min_refresh_interval: Duration::from_millis(250),
        }
    }
}

pub struct MemoryLimiter {
    settings: MemoryLimiterSettings,
    last_snapshot: Option<(Instant, MemorySnapshot)>,
}

impl MemoryLimiter {
    pub fn new(settings: MemoryLimiterSettings) -> Self {
        Self { settings, last_snapshot: None }
    }

    pub fn above_high(&mut self) -> bool {
        let snap = self.snapshot();
        if let Some(snap) = snap {
            if snap.total_bytes > 0 {
                let used_ratio = (snap.used_bytes as f64) / (snap.total_bytes as f64);
                return used_ratio >= self.settings.high_watermark_ratio;
            }
        }
        false
    }

    pub fn below_low(&mut self) -> bool {
        let snap = self.snapshot();
        if let Some(snap) = snap {
            if snap.total_bytes > 0 {
                let used_ratio = (snap.used_bytes as f64) / (snap.total_bytes as f64);
                return used_ratio <= self.settings.low_watermark_ratio;
            }
        }
        false
    }

    pub fn snapshot(&mut self) -> Option<MemorySnapshot> {
        let now = Instant::now();
        if let Some((t, snap)) = self.last_snapshot {
            if now.duration_since(t) < self.settings.min_refresh_interval {
                return Some(snap);
            }
        }

        let snap = read_memory_snapshot();
        if let Some(s) = snap {
            self.last_snapshot = Some((now, s));
        }
        snap
    }
}

fn read_memory_snapshot() -> Option<MemorySnapshot> {
    // Try cgroup v2 memory limits first (common in containers/k8s)
    if let Some(cg) = read_cgroup_v2_memory() {
        return Some(cg);
    }

    // Fall back to system MemTotal/MemAvailable and process RSS
    let (total, available) = read_proc_meminfo()?;
    let rss = read_process_rss_bytes().unwrap_or(0);
    let used = total.saturating_sub(available);
    Some(MemorySnapshot { total_bytes: total, used_bytes: used, process_rss_bytes: rss })
}

fn read_cgroup_v2_memory() -> Option<MemorySnapshot> {
    // memory.max may be "max" if unlimited
    let max_path = "/sys/fs/cgroup/memory.max";
    let current_path = "/sys/fs/cgroup/memory.current";
    let limit = fs::read_to_string(max_path).ok()?.trim().to_string();
    let current = fs::read_to_string(current_path).ok()?.trim().to_string();

    let current_bytes: u64 = current.parse().ok()?;
    let total_bytes: u64 = if limit == "max" { 0 } else { limit.parse().ok()? };

    // If total is unknown (unlimited), derive from host MemTotal
    let total_bytes = if total_bytes == 0 {
        read_proc_meminfo()?.0
    } else {
        total_bytes
    };

    let rss = read_process_rss_bytes().unwrap_or(0);
    Some(MemorySnapshot { total_bytes, used_bytes: current_bytes, process_rss_bytes: rss })
}

fn read_proc_meminfo() -> Option<(u64, u64)> {
    let content = fs::read_to_string("/proc/meminfo").ok()?;
    let mut total_kb: Option<u64> = None;
    let mut available_kb: Option<u64> = None;
    for line in content.lines() {
        if line.starts_with("MemTotal:") {
            total_kb = extract_kb(line);
        } else if line.starts_with("MemAvailable:") {
            available_kb = extract_kb(line);
        }
        if total_kb.is_some() && available_kb.is_some() {
            break;
        }
    }
    let total = total_kb? * 1024;
    let available = available_kb? * 1024;
    Some((total, available))
}

fn extract_kb(line: &str) -> Option<u64> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() >= 2 {
        return parts[1].parse::<u64>().ok();
    }
    None
}

fn read_process_rss_bytes() -> Option<u64> {
    let content = fs::read_to_string("/proc/self/status").ok()?;
    for line in content.lines() {
        if line.starts_with("VmRSS:") {
            if let Some(kb) = extract_kb(line) {
                return Some(kb * 1024);
            }
        }
    }
    None
}


