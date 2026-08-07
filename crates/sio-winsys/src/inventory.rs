//! Read-only system inventory for the dashboard.
//!
//! Nothing in this module writes anything. In particular the activation status is
//! *reported* — SIO never touches licensing.

use sio_core::error::Result;
use sio_core::sysinfo::{
    ActivationStatus, CpuInfo, DiskInfo, DiskKind, GpuInfo, MemoryInfo, OsInfo, SystemSnapshot,
};

#[cfg(windows)]
use crate::registry;

const CURRENT_VERSION_KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";

/// Collect a full snapshot.
///
/// Individual probes degrade to sensible unknowns rather than failing the whole
/// dashboard — a machine with an exotic GPU driver should still show its CPU.
pub async fn probe() -> Result<SystemSnapshot> {
    let (cpu, memory, disks) = hardware();

    Ok(SystemSnapshot {
        os: os_info().await,
        cpu,
        memory,
        gpus: gpus(),
        disks,
    })
}

fn hardware() -> (CpuInfo, MemoryInfo, Vec<DiskInfo>) {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    sys.refresh_cpu_list(sysinfo::CpuRefreshKind::nothing());

    let cpu = CpuInfo {
        brand: sys
            .cpus()
            .first()
            .map(|c| c.brand().trim().to_string())
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| "Unknown CPU".to_string()),
        physical_cores: sysinfo::System::physical_core_count(),
        logical_cores: sys.cpus().len(),
    };

    let memory = MemoryInfo {
        total_bytes: sys.total_memory(),
        used_bytes: sys.used_memory(),
    };

    let disks = sysinfo::Disks::new_with_refreshed_list()
        .iter()
        .map(|d| DiskInfo {
            mount_point: d
                .mount_point()
                .to_string_lossy()
                .trim_end_matches('\\')
                .to_string(),
            total_bytes: d.total_space(),
            available_bytes: d.available_space(),
            is_removable: d.is_removable(),
            kind: match d.kind() {
                sysinfo::DiskKind::SSD => DiskKind::Ssd,
                sysinfo::DiskKind::HDD => DiskKind::Hdd,
                _ => DiskKind::Unknown,
            },
        })
        .collect();

    (cpu, memory, disks)
}

#[cfg(windows)]
async fn os_info() -> OsInfo {
    use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;

    let read = |name: &str| {
        registry::read_string(HKEY_LOCAL_MACHINE, CURRENT_VERSION_KEY, name)
            .ok()
            .flatten()
    };

    let build = read("CurrentBuildNumber")
        .and_then(|b| b.parse::<u32>().ok())
        .unwrap_or(0);

    // Windows 11 still reports "Windows 10 ..." in ProductName — a well-known quirk
    // Microsoft never fixed. Correct it from the build number rather than showing the
    // user the wrong OS name on their own dashboard.
    let product = read("ProductName").unwrap_or_else(|| "Windows".to_string());
    let edition = if build >= 22000 && product.contains("Windows 10") {
        product.replace("Windows 10", "Windows 11")
    } else {
        product
    };

    OsInfo {
        edition,
        display_version: read("DisplayVersion")
            .or_else(|| read("ReleaseId"))
            .unwrap_or_else(|| "Unknown".to_string()),
        build,
        arch: std::env::consts::ARCH.to_string(),
        machine_name: sysinfo::System::host_name().unwrap_or_else(|| "Unknown".to_string()),
        activation: activation_status().await,
    }
}

#[cfg(not(windows))]
async fn os_info() -> OsInfo {
    OsInfo {
        edition: "Non-Windows host".to_string(),
        display_version: "n/a".to_string(),
        build: 0,
        arch: std::env::consts::ARCH.to_string(),
        machine_name: sysinfo::System::host_name().unwrap_or_else(|| "Unknown".to_string()),
        activation: ActivationStatus::Unknown,
    }
}

/// Query Windows licensing status.
///
/// Uses CIM rather than parsing `slmgr` output, because `slmgr` prints localized text.
/// `LicenseStatus` is a stable numeric enum, so this works identically on a Russian or
/// English install. Any failure reports `Unknown` — never a guess.
#[cfg(windows)]
async fn activation_status() -> ActivationStatus {
    const WINDOWS_APP_ID: &str = "55c92734-d682-4d71-983e-d6ec3f16059f";
    let script = format!(
        "(Get-CimInstance SoftwareLicensingProduct -Filter \"ApplicationID='{WINDOWS_APP_ID}' \
         AND PartialProductKey IS NOT NULL\" | Select-Object -First 1).LicenseStatus"
    );

    let output = tokio::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .await;

    let Ok(output) = output else {
        return ActivationStatus::Unknown;
    };

    match String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u32>()
    {
        Ok(0) => ActivationStatus::Unlicensed,
        Ok(1) => ActivationStatus::Licensed,
        // 2 = out-of-box grace, 3 = out-of-tolerance grace, 4 = non-genuine grace.
        Ok(2..=4) => ActivationStatus::GracePeriod,
        Ok(5) => ActivationStatus::Notification,
        _ => ActivationStatus::Unknown,
    }
}

#[cfg(not(windows))]
async fn activation_status() -> ActivationStatus {
    ActivationStatus::Unknown
}

/// Suppress the console window that would otherwise flash on every child process.
///
/// Without this, every `powershell` call blinks a black window on top of the app.
#[cfg(windows)]
pub(crate) const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Enumerate physical display adapters via DXGI.
///
/// DXGI is used rather than a WMI query because it is in-process and returns instantly,
/// where spawning a CIM query costs a few hundred milliseconds on the dashboard's
/// critical path.
#[cfg(windows)]
fn gpus() -> Vec<GpuInfo> {
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG, DXGI_ADAPTER_FLAG_SOFTWARE,
    };

    let mut out = Vec::new();
    unsafe {
        let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else {
            return out;
        };

        for index in 0.. {
            let Ok(adapter) = factory.EnumAdapters1(index) else {
                break;
            };
            let Ok(desc) = adapter.GetDesc1() else {
                continue;
            };
            // Skip the Microsoft Basic Render Driver — it is not a real GPU.
            if DXGI_ADAPTER_FLAG(desc.Flags as i32) == DXGI_ADAPTER_FLAG_SOFTWARE {
                continue;
            }

            let end = desc
                .Description
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(desc.Description.len());
            let name = String::from_utf16_lossy(&desc.Description[..end])
                .trim()
                .to_string();
            if name.is_empty() {
                continue;
            }

            out.push(GpuInfo {
                name,
                vram_bytes: (desc.DedicatedVideoMemory > 0)
                    .then_some(desc.DedicatedVideoMemory as u64),
                driver_version: None,
            });
        }
    }
    out
}

#[cfg(not(windows))]
fn gpus() -> Vec<GpuInfo> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn probe_returns_plausible_hardware() {
        let snap = probe()
            .await
            .expect("probe should not fail on a healthy system");

        assert!(
            snap.cpu.logical_cores >= 1,
            "every machine has at least one logical core"
        );
        assert!(!snap.cpu.brand.is_empty());
        assert!(snap.memory.total_bytes > 0, "total memory must be readable");
        assert!(
            snap.memory.used_bytes <= snap.memory.total_bytes,
            "used memory cannot exceed total"
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn reports_a_real_windows_build() {
        let snap = probe().await.unwrap();

        assert!(
            snap.os.build >= 10240,
            "expected a Windows 10+ build, got {}",
            snap.os.build
        );
        assert!(
            snap.os.edition.to_lowercase().contains("windows"),
            "got {}",
            snap.os.edition
        );

        // The ProductName quirk fix: an 11 build must never be labelled "Windows 10".
        if snap.os.is_windows_11() {
            assert!(
                !snap.os.edition.contains("Windows 10"),
                "build {} was labelled {:?}",
                snap.os.build,
                snap.os.edition
            );
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn finds_the_system_drive() {
        let snap = probe().await.unwrap();
        assert!(
            snap.disks.iter().any(|d| d.mount_point.starts_with('C')),
            "expected a C: drive among {:?}",
            snap.disks
                .iter()
                .map(|d| &d.mount_point)
                .collect::<Vec<_>>()
        );
    }
}
