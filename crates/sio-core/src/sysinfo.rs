//! System inventory presented on the dashboard.
//!
//! Every field here is read-only. Notably `activation` is *reported*, never altered.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemSnapshot {
    pub os: OsInfo,
    pub cpu: CpuInfo,
    pub memory: MemoryInfo,
    pub gpus: Vec<GpuInfo>,
    pub disks: Vec<DiskInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OsInfo {
    /// e.g. "Windows 11 Pro"
    pub edition: String,
    /// e.g. "24H2"
    pub display_version: String,
    /// e.g. 26100
    pub build: u32,
    pub arch: String,
    pub machine_name: String,
    pub activation: ActivationStatus,
}

impl OsInfo {
    /// Windows 11 is build 22000 and above; everything below is Windows 10.
    ///
    /// Some tweaks target only one of the two, so the tweak engine gates on this.
    pub fn is_windows_11(&self) -> bool {
        self.build >= 22000
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivationStatus {
    Licensed,
    GracePeriod,
    Notification,
    Unlicensed,
    /// Could not be determined — shown as "unknown", never guessed at.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CpuInfo {
    pub brand: String,
    pub physical_cores: Option<usize>,
    pub logical_cores: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryInfo {
    pub total_bytes: u64,
    pub used_bytes: u64,
}

impl MemoryInfo {
    pub fn used_percent(&self) -> f32 {
        if self.total_bytes == 0 {
            return 0.0;
        }
        (self.used_bytes as f64 / self.total_bytes as f64 * 100.0) as f32
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuInfo {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vram_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskInfo {
    /// e.g. "C:"
    pub mount_point: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub is_removable: bool,
    pub kind: DiskKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiskKind {
    Ssd,
    Hdd,
    Unknown,
}

impl DiskInfo {
    pub fn used_bytes(&self) -> u64 {
        self.total_bytes.saturating_sub(self.available_bytes)
    }

    pub fn used_percent(&self) -> f32 {
        if self.total_bytes == 0 {
            return 0.0;
        }
        (self.used_bytes() as f64 / self.total_bytes as f64 * 100.0) as f32
    }

    /// Below this, a bulk install of a large profile is likely to fail partway.
    pub fn is_low_on_space(&self) -> bool {
        const WARN_THRESHOLD: u64 = 10 * 1024 * 1024 * 1024; // 10 GiB
        self.available_bytes < WARN_THRESHOLD
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disk(total: u64, available: u64) -> DiskInfo {
        DiskInfo {
            mount_point: "C:".into(),
            total_bytes: total,
            available_bytes: available,
            is_removable: false,
            kind: DiskKind::Ssd,
        }
    }

    #[test]
    fn windows_11_detection_uses_the_22000_boundary() {
        let mut os = OsInfo {
            edition: "Windows 10 Pro".into(),
            display_version: "22H2".into(),
            build: 19045,
            arch: "x86_64".into(),
            machine_name: "PC".into(),
            activation: ActivationStatus::Licensed,
        };
        assert!(!os.is_windows_11());

        os.build = 22000;
        assert!(os.is_windows_11(), "22000 is the first Windows 11 build");
    }

    #[test]
    fn disk_usage_maths() {
        let d = disk(100, 25);
        assert_eq!(d.used_bytes(), 75);
        assert_eq!(d.used_percent(), 75.0);
    }

    #[test]
    fn zero_sized_disk_does_not_divide_by_zero() {
        let d = disk(0, 0);
        assert_eq!(d.used_percent(), 0.0);
        assert_eq!(d.used_bytes(), 0);
    }

    #[test]
    fn available_greater_than_total_saturates_instead_of_underflowing() {
        // Guards against a reported-size quirk panicking the dashboard in release.
        let d = disk(100, 200);
        assert_eq!(d.used_bytes(), 0);
    }

    #[test]
    fn low_space_warning_triggers_under_10_gib() {
        assert!(disk(500 * 1024 * 1024 * 1024, 5 * 1024 * 1024 * 1024).is_low_on_space());
        assert!(!disk(500 * 1024 * 1024 * 1024, 50 * 1024 * 1024 * 1024).is_low_on_space());
    }

    #[test]
    fn memory_percent_handles_zero_total() {
        assert_eq!(
            MemoryInfo {
                total_bytes: 0,
                used_bytes: 0
            }
            .used_percent(),
            0.0
        );
    }
}
