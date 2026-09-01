//! Server system vitals — board #56 「服务端系统状态显示」.
//!
//! The owner wants three numbers in the desktop client's bottom-left corner:
//! CPU, memory, and disk, at a LOW refresh rate ("可以不用很高刷新率 大概看个
//! 数就行"). This module is the whole collection half: one `read()` per
//! `system_status` RPC call, no background thread, no timer — the CLIENT's
//! poll interval is the clock, which is what makes low frequency free.
//!
//! CPU% needs a delta window (two samples of busy/total time). Instead of
//! sleeping inside the request (a 200ms stall per poll) the previous sample
//! is KEPT between calls in a process-wide `System`, so each reading reports
//! usage since the LAST poll — exactly the interval the client chose. The
//! price is honest: the FIRST call has no window and reports `cpu_pct: null`
//! rather than a made-up 0% (fail-soft: the client renders the other numbers
//! and fills CPU in on its next tick).
//!
//! Disk is the ROOT filesystem — `pick_root_disk` prefers the exact `/`
//! mount and otherwise falls back to the largest mounted disk (macOS lists
//! the APFS data volume separately; largest-total is the container the owner
//! means by "disk容量"). Everything degrades soft: an empty disk list yields
//! 0/0, which the client treats as "nothing to say" — a failed reading must
//! never break the connection that carries the terminal.
//!
//! No shell-outs anywhere (the review brief bans slow commands and injection
//! surfaces): `sysinfo` reads /proc on Linux and the host statistics API on
//! macOS, in-process.

use serde::Serialize;
use std::sync::{Mutex, OnceLock};
use sysinfo::{Disks, System};

/// One reading. Bytes everywhere; percentages are 0..=100.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct SystemStatus {
    /// Whole-machine CPU usage since the previous poll; `None` on the first
    /// call of a server's life (no delta window exists yet).
    pub cpu_pct: Option<f32>,
    pub mem_used: u64,
    pub mem_total: u64,
    pub disk_used: u64,
    pub disk_total: u64,
}

/// The process-wide sampler. `bool` = "a previous CPU sample exists", i.e.
/// whether the usage sysinfo reports has a real window under it.
static SYS: OnceLock<Mutex<(System, bool)>> = OnceLock::new();

/// Take one reading. Cheap enough for every poll: a /proc read (Linux) or a
/// couple of host calls (macOS) plus a disk-list refresh.
pub fn read() -> SystemStatus {
    let lock = SYS.get_or_init(|| Mutex::new((System::new(), false)));
    let (cpu_pct, mem_used, mem_total) = match lock.lock() {
        Ok(mut guard) => {
            let (sys, primed) = &mut *guard;
            sys.refresh_cpu_usage();
            sys.refresh_memory();
            let pct = if *primed {
                Some(sys.global_cpu_usage().clamp(0.0, 100.0))
            } else {
                *primed = true;
                None
            };
            (pct, sys.used_memory(), sys.total_memory())
        }
        // A poisoned lock means a past panic mid-read; report "no reading"
        // rather than poisoning every future poll too.
        Err(_) => (None, 0, 0),
    };

    let disks = Disks::new_with_refreshed_list();
    let listed: Vec<(String, u64, u64)> = disks
        .iter()
        .map(|d| {
            (
                d.mount_point().to_string_lossy().into_owned(),
                d.total_space(),
                d.available_space(),
            )
        })
        .collect();
    let (disk_total, disk_avail) = pick_root_disk(&listed).unwrap_or((0, 0));

    SystemStatus {
        cpu_pct,
        mem_used,
        mem_total,
        disk_used: disk_total.saturating_sub(disk_avail),
        disk_total,
    }
}

/// Which mounted disk is "the disk" (total, available)? Exact `/` when it is
/// listed (Linux, and macOS's synthetic root), else the largest total —
/// macOS splits the APFS container across read-only `/` and the data volume,
/// and the largest one is the capacity a person means. Pure so the choice is
/// testable without a platform.
pub fn pick_root_disk(disks: &[(String, u64, u64)]) -> Option<(u64, u64)> {
    if let Some(root) = disks.iter().find(|(m, ..)| m == "/") {
        if root.1 > 0 {
            return Some((root.1, root.2));
        }
    }
    disks
        .iter()
        .max_by_key(|(_, total, _)| *total)
        .filter(|(_, total, _)| *total > 0)
        .map(|(_, total, avail)| (*total, *avail))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_read_has_no_cpu_window_later_reads_do() {
        // Same process-wide sampler as production: the FIRST call must say
        // "no reading yet" instead of a fabricated 0%, and a later call must
        // report a real bounded percentage. (Order within this test does not
        // depend on other tests: whichever call is globally first returns
        // None, every call after it returns Some.)
        let a = read();
        let b = read();
        assert!(b.cpu_pct.is_some(), "second read has a delta window");
        let pct = b.cpu_pct.unwrap();
        assert!((0.0..=100.0).contains(&pct), "cpu% bounded, got {pct}");
        // Memory always reads on a live machine.
        assert!(a.mem_total > 0 && a.mem_used <= a.mem_total);
    }

    #[test]
    fn root_disk_prefers_slash_over_larger_siblings() {
        let disks = vec![
            ("/data".to_string(), 900, 400),
            ("/".to_string(), 500, 200),
        ];
        assert_eq!(pick_root_disk(&disks), Some((500, 200)));
    }

    #[test]
    fn no_slash_falls_back_to_largest_and_empty_is_none() {
        let disks = vec![
            ("/System/Volumes/Data".to_string(), 994_000, 400_000),
            ("/System/Volumes/VM".to_string(), 1_000, 900),
        ];
        assert_eq!(pick_root_disk(&disks), Some((994_000, 400_000)));
        assert_eq!(pick_root_disk(&[]), None);
        // A zero-sized "/" (a broken statvfs) must not win either.
        assert_eq!(pick_root_disk(&[("/".to_string(), 0, 0)]), None);
    }

    #[test]
    fn wire_shape_is_the_documented_contract() {
        // The client's formatter is written against these exact keys.
        let s = SystemStatus {
            cpu_pct: Some(12.5),
            mem_used: 3,
            mem_total: 16,
            disk_used: 210,
            disk_total: 500,
        };
        let v = serde_json::to_value(&s).unwrap();
        for k in ["cpu_pct", "mem_used", "mem_total", "disk_used", "disk_total"] {
            assert!(v.get(k).is_some(), "missing key {k}");
        }
        let first = SystemStatus { cpu_pct: None, ..s };
        assert!(serde_json::to_value(&first).unwrap()["cpu_pct"].is_null());
    }
}
