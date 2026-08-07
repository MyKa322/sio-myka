//! System Restore points and Appx (UWP) package removal.
//!
//! Both go through PowerShell rather than raw COM: the underlying interfaces are WMI
//! and the Appx deployment API, and driving either from Rust costs far more code than
//! it saves. All decisions come from structured JSON or exit codes, never from
//! localized text.

use crate::process::run_captured;
use sio_core::error::{Error, Result};
use sio_core::privileged::RestorePointOutcome;
use sio_core::tweak::AppxRef;

fn powershell(script: &str) -> Vec<String> {
    vec![
        "-NoProfile".into(),
        "-NonInteractive".into(),
        "-ExecutionPolicy".into(),
        "Bypass".into(),
        "-Command".into(),
        script.into(),
    ]
}

/// `ERROR_SERVICE_DISABLED` — what WMI returns when System Protection is turned off.
const ERROR_SERVICE_DISABLED: i64 = 1058;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RestorePointReport {
    return_value: i64,
    created: bool,
    #[serde(default)]
    sequence: Option<u64>,
}

/// Create a System Restore point.
///
/// Detection is done by comparing the restore-point list before and after, not by
/// trusting the return code alone. Windows throttles restore points to roughly one per
/// 24 hours and will report success while creating nothing — so a tool that trusts the
/// return value tells the user they have a safety net when they do not.
pub async fn create_restore_point(description: &str) -> Result<RestorePointOutcome> {
    // Single-quoted and doubled inside PowerShell so a description can never break out
    // of the literal.
    let safe_description = description.replace('\'', "''");

    let script = format!(
        r#"
$ErrorActionPreference = 'SilentlyContinue'
$before = @(Get-ComputerRestorePoint | Select-Object -ExpandProperty SequenceNumber)
$rv = -1
try {{
    $r = Invoke-CimMethod -Namespace root/default -ClassName SystemRestore `
        -MethodName CreateRestorePoint `
        -Arguments @{{ Description = '{safe_description}'; RestorePointType = [uint32]12; EventType = [uint32]100 }} -ErrorAction Stop
    $rv = [int]$r.ReturnValue
}} catch {{
    if ($_.Exception.HResult) {{ $rv = 1058 }} else {{ $rv = -2 }}
}}
$after = @(Get-ComputerRestorePoint | Select-Object -ExpandProperty SequenceNumber)
$new = @($after | Where-Object {{ $before -notcontains $_ }})
[pscustomobject]@{{
    returnValue = $rv
    created     = ($new.Count -gt 0)
    sequence    = if ($new.Count -gt 0) {{ [uint64]($new | Sort-Object | Select-Object -Last 1) }} else {{ $null }}
}} | ConvertTo-Json -Compress
"#
    );

    let (_, stdout) = run_captured("powershell", &powershell(&script)).await?;

    let report: RestorePointReport = serde_json::from_str(stdout.trim()).map_err(|e| {
        Error::Other(format!(
            "could not read the restore-point result: {e}; output was {stdout:?}"
        ))
    })?;

    if report.created {
        return Ok(RestorePointOutcome::Created {
            sequence_number: report.sequence.unwrap_or(0),
        });
    }
    if report.return_value == ERROR_SERVICE_DISABLED {
        return Ok(RestorePointOutcome::SkippedDisabled);
    }
    if report.return_value == 0 {
        return Ok(RestorePointOutcome::SkippedThrottled);
    }

    Err(Error::Other(format!(
        "creating a restore point failed with code {}",
        report.return_value
    )))
}

/// Packages that must never be removed, whatever a catalog says.
///
/// These are frameworks and shell components rather than apps: taking them out breaks
/// the Store, the Start menu, or every app built on the C++ runtime. The catalog is
/// fetched from the network, so this list is a hard backstop in code — not advice.
const NEVER_REMOVE: &[&str] = &[
    "Microsoft.VCLibs",
    "Microsoft.NET.Native",
    "Microsoft.UI.Xaml",
    "Microsoft.WindowsStore",
    "Microsoft.StorePurchaseApp",
    "Microsoft.DesktopAppInstaller",
    "Microsoft.WindowsAppRuntime",
    "Microsoft.ShellExperienceHost",
    "Microsoft.Windows.ShellExperienceHost",
    "Microsoft.Windows.StartMenuExperienceHost",
    "Microsoft.Windows.SecHealthUI",
    "Microsoft.SecHealthUI",
    "Microsoft.AccountsControl",
    "Microsoft.Windows.CloudExperienceHost",
    "Microsoft.Windows.ContentDeliveryManager",
    "Microsoft.CredDialogHost",
    "Microsoft.LockApp",
    "Microsoft.Win32WebViewHost",
    "Microsoft.MicrosoftEdge",
    "Microsoft.WebpImageExtension",
    "Microsoft.HEIFImageExtension",
];

/// Whether a package family name is protected from removal.
pub fn is_protected(package_family_name: &str) -> bool {
    NEVER_REMOVE.iter().any(|blocked| {
        package_family_name.len() >= blocked.len()
            && package_family_name[..blocked.len()].eq_ignore_ascii_case(blocked)
    })
}

/// Remove a UWP package for the current user, optionally deprovisioning it so new user
/// profiles do not get it back.
///
/// Not reversible by us: reinstalling requires the Store. The journal records the
/// removal so the user can be told exactly what went.
pub async fn appx_remove(package: &AppxRef) -> Result<()> {
    if is_protected(&package.package_family_name) {
        return Err(Error::Other(format!(
            "`{}` is a protected system component and will not be removed",
            package.package_family_name
        )));
    }

    let pfn = package.package_family_name.replace('\'', "''");
    let mut script = format!(
        r#"
$ErrorActionPreference = 'Stop'
$pkg = Get-AppxPackage | Where-Object {{ $_.PackageFamilyName -eq '{pfn}' }}
if ($pkg) {{ $pkg | Remove-AppxPackage }}
"#
    );

    if package.deprovision {
        script.push_str(&format!(
            r#"
$prov = Get-AppxProvisionedPackage -Online | Where-Object {{ $_.PackageName -like '{}*' }}
foreach ($p in $prov) {{ Remove-AppxProvisionedPackage -Online -PackageName $p.PackageName | Out-Null }}
"#,
            pfn.split('_').next().unwrap_or(&pfn)
        ));
    }

    let (code, _) = run_captured("powershell", &powershell(&script)).await?;
    if code != 0 {
        return Err(Error::Other(format!(
            "removing `{}` failed with exit code {code}",
            package.package_family_name
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_frameworks_are_protected() {
        assert!(is_protected("Microsoft.VCLibs.140.00_8wekyb3d8bbwe"));
        assert!(is_protected(
            "Microsoft.NET.Native.Framework.2.2_8wekyb3d8bbwe"
        ));
        assert!(is_protected("Microsoft.WindowsStore_8wekyb3d8bbwe"));
        assert!(is_protected("Microsoft.UI.Xaml.2.8_8wekyb3d8bbwe"));
    }

    #[test]
    fn shell_components_are_protected() {
        assert!(is_protected(
            "Microsoft.Windows.StartMenuExperienceHost_cw5n1h2txyewy"
        ));
        assert!(is_protected("Microsoft.SecHealthUI_8wekyb3d8bbwe"));
    }

    #[test]
    fn protection_is_case_insensitive() {
        // A catalog entry with different casing must not slip past the backstop.
        assert!(is_protected("microsoft.windowsstore_8wekyb3d8bbwe"));
        assert!(is_protected("MICROSOFT.VCLIBS.140.00_8wekyb3d8bbwe"));
    }

    #[test]
    fn ordinary_bloat_is_not_protected() {
        assert!(!is_protected("Microsoft.BingNews_8wekyb3d8bbwe"));
        assert!(!is_protected("Microsoft.XboxGamingOverlay_8wekyb3d8bbwe"));
        assert!(!is_protected("Clipchamp.Clipchamp_yxz26nhyzhsrt"));
    }

    #[tokio::test]
    async fn removing_a_protected_package_is_refused_before_powershell_runs() {
        let err = appx_remove(&AppxRef {
            package_family_name: "Microsoft.WindowsStore_8wekyb3d8bbwe".into(),
            deprovision: true,
        })
        .await
        .unwrap_err();

        assert!(err.to_string().contains("protected"), "got {err}");
    }

    /// Creating a restore point needs elevation, is rate-limited to one per day, and
    /// changes the machine. Run deliberately on a VM.
    #[tokio::test]
    #[ignore = "requires elevation and modifies the system"]
    async fn create_a_real_restore_point() {
        let outcome = create_restore_point("SIO test").await.unwrap();
        // Any of the three is a legitimate answer depending on machine state; what
        // matters is that we classify rather than crash or lie.
        println!("restore point outcome: {outcome:?}");
    }
}
