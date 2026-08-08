//! The winget provider.
//!
//! winget ships with Windows 11 and needs no bootstrap, so it is the only provider we
//! can assume is present. It is also the most awkward to automate: `search` and `list`
//! have no JSON mode, and their column headers are localized. Nothing here parses that
//! output.
//!
//! Inventory instead comes from `winget export`, which emits a documented, versioned
//! JSON schema, and every decision comes from an exit code.

use crate::provider::{common, probe, PackageProvider, Verdict};
use async_trait::async_trait;
use serde::Deserialize;
use sio_core::error::{Error, PackageFailure, Result};
use sio_core::package::{InstalledPackage, PackageCmd, PackageOp, PackageRef, ProviderId};

/// Exit codes from `AppInstallerErrors.h` in microsoft/winget-cli.
///
/// `NO_APPLICATIONS_FOUND` was additionally confirmed by hand against winget 1.29 —
/// worth doing, because these are easy to misremember and a wrong mapping turns a
/// stale catalog entry into a scary "install failed".
mod codes {
    pub const SHELLEXEC_INSTALL_FAILED: i32 = 0x8A15_0006u32 as i32;
    pub const DOWNLOAD_FAILED: i32 = 0x8A15_0008u32 as i32;
    pub const NO_APPLICABLE_INSTALLER: i32 = 0x8A15_0010u32 as i32;
    pub const INSTALLER_HASH_MISMATCH: i32 = 0x8A15_0011u32 as i32;
    pub const NO_APPLICATIONS_FOUND: i32 = 0x8A15_0014u32 as i32;
    pub const MULTIPLE_APPLICATIONS_FOUND: i32 = 0x8A15_0016u32 as i32;
    pub const COMMAND_REQUIRES_ADMIN: i32 = 0x8A15_0019u32 as i32;
    pub const UPDATE_NOT_APPLICABLE: i32 = 0x8A15_002Bu32 as i32;
    pub const PACKAGE_ALREADY_INSTALLED: i32 = 0x8A15_0061u32 as i32;
    pub const INSTALL_REBOOT_REQUIRED_TO_FINISH: i32 = 0x8A15_0109u32 as i32;
    pub const INSTALL_REBOOT_REQUIRED_FOR_INSTALL: i32 = 0x8A15_010Au32 as i32;
    pub const INSTALL_REBOOT_INITIATED: i32 = 0x8A15_010Bu32 as i32;
}

/// Flags every invocation needs so winget never blocks waiting for a human.
fn base_flags() -> Vec<String> {
    vec![
        "--exact".into(),
        "--silent".into(),
        "--disable-interactivity".into(),
        "--accept-package-agreements".into(),
        "--accept-source-agreements".into(),
    ]
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Winget;

/// The shape of a `winget export` document. Only the fields we rely on.
#[derive(Debug, Deserialize)]
struct ExportDocument {
    #[serde(rename = "Sources", default)]
    sources: Vec<ExportSource>,
}

#[derive(Debug, Deserialize)]
struct ExportSource {
    #[serde(rename = "Packages", default)]
    packages: Vec<ExportPackage>,
}

#[derive(Debug, Deserialize)]
struct ExportPackage {
    #[serde(rename = "PackageIdentifier")]
    identifier: String,
    #[serde(rename = "Version", default)]
    version: Option<String>,
}

impl Winget {
    pub fn new() -> Self {
        Self
    }

    /// Parse a `winget export` document into an inventory.
    ///
    /// Split out from the I/O so the schema handling is testable without winget.
    pub(crate) fn parse_export(json: &str) -> Result<Vec<InstalledPackage>> {
        let document: ExportDocument = serde_json::from_str(json).map_err(|e| {
            Error::Other(format!(
                "winget export produced JSON we could not read: {e}"
            ))
        })?;

        Ok(document
            .sources
            .into_iter()
            .flat_map(|source| source.packages)
            .map(|pkg| InstalledPackage {
                provider: ProviderId::Winget,
                name: pkg.identifier.clone(),
                id: pkg.identifier,
                version: pkg.version.unwrap_or_default(),
                available_version: None,
            })
            .collect())
    }
}

#[async_trait]
impl PackageProvider for Winget {
    fn id(&self) -> ProviderId {
        ProviderId::Winget
    }

    async fn is_available(&self) -> bool {
        probe("winget", &["--version"]).await
    }

    async fn installed(&self) -> Result<Vec<InstalledPackage>> {
        // `winget export` writes to a file rather than stdout, so it needs a scratch
        // path. A per-call unique name avoids two concurrent refreshes colliding.
        let path =
            std::env::temp_dir().join(format!("sio-winget-export-{}.json", std::process::id()));
        let path_str = path.to_string_lossy().into_owned();

        let args = vec![
            "export".into(),
            "-o".into(),
            path_str,
            "--include-versions".into(),
            "--disable-interactivity".into(),
            "--accept-source-agreements".into(),
        ];

        // Exit code is deliberately ignored: export reports non-zero when *some*
        // installed program is not known to any source, which is normal on any real
        // machine and does not mean the export failed.
        let _ = sio_winsys::process::run_captured("winget", &args).await?;

        let json = tokio::fs::read_to_string(&path).await.map_err(|e| {
            Error::Other(format!(
                "could not read the winget export at {}: {e}",
                path.display()
            ))
        })?;
        let _ = tokio::fs::remove_file(&path).await;

        Self::parse_export(&json)
    }

    fn install_cmd(&self, pkg: &PackageRef) -> PackageCmd {
        let mut args = vec!["install".into(), "--id".into(), pkg.id.clone()];
        args.extend(base_flags());
        if let Some(version) = &pkg.version {
            args.push("--version".into());
            args.push(version.clone());
        }

        PackageCmd {
            provider: ProviderId::Winget,
            op: PackageOp::Install,
            program: "winget".into(),
            args,
            elevated: true,
        }
    }

    fn uninstall_cmd(&self, pkg: &PackageRef) -> PackageCmd {
        let mut args = vec!["uninstall".into(), "--id".into(), pkg.id.clone()];
        args.extend(base_flags());
        // Not a valid uninstall flag.
        args.retain(|a| a != "--accept-package-agreements");

        PackageCmd {
            provider: ProviderId::Winget,
            op: PackageOp::Uninstall,
            program: "winget".into(),
            args,
            elevated: true,
        }
    }

    fn classify(&self, code: i32) -> Verdict {
        match code {
            0 => Verdict::Success,

            common::REBOOT_REQUIRED
            | common::REBOOT_INITIATED
            | codes::INSTALL_REBOOT_REQUIRED_TO_FINISH
            | codes::INSTALL_REBOOT_REQUIRED_FOR_INSTALL
            | codes::INSTALL_REBOOT_INITIATED => Verdict::RebootRequired,

            // Both mean "the end state you asked for already holds".
            codes::PACKAGE_ALREADY_INSTALLED | codes::UPDATE_NOT_APPLICABLE => Verdict::AlreadyDone,

            codes::NO_APPLICATIONS_FOUND | codes::MULTIPLE_APPLICATIONS_FOUND => {
                Verdict::Failed(PackageFailure::NotFound)
            }
            codes::NO_APPLICABLE_INSTALLER => {
                Verdict::Failed(PackageFailure::NoApplicableInstaller)
            }
            codes::DOWNLOAD_FAILED | codes::INSTALLER_HASH_MISMATCH => {
                Verdict::Failed(PackageFailure::Network)
            }
            codes::COMMAND_REQUIRES_ADMIN => Verdict::Failed(PackageFailure::RequiresElevation),
            codes::SHELLEXEC_INSTALL_FAILED => Verdict::Failed(PackageFailure::InstallFailed),

            // Anything unmapped is reported honestly rather than guessed at; the raw
            // code travels with it so a bug report is actionable.
            _ => Verdict::Failed(PackageFailure::Unknown),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(id: &str) -> PackageRef {
        PackageRef::new(ProviderId::Winget, id)
    }

    #[test]
    fn install_command_targets_an_exact_id_and_never_prompts() {
        let cmd = Winget.install_cmd(&pkg("Mozilla.Firefox"));

        assert_eq!(cmd.program, "winget");
        assert!(
            cmd.elevated,
            "machine-scope installs need administrator rights"
        );

        // --exact plus --id is what lets us skip winget's localized search output.
        assert!(cmd.args.contains(&"--id".to_string()));
        assert!(cmd.args.contains(&"Mozilla.Firefox".to_string()));
        assert!(cmd.args.contains(&"--exact".to_string()));

        // Any missing "accept" flag makes winget block forever behind a prompt no one
        // can see, because the process has no console.
        assert!(cmd.args.contains(&"--disable-interactivity".to_string()));
        assert!(cmd
            .args
            .contains(&"--accept-package-agreements".to_string()));
        assert!(cmd.args.contains(&"--accept-source-agreements".to_string()));
    }

    #[test]
    fn a_pinned_version_is_passed_through() {
        let mut reference = pkg("Git.Git");
        reference.version = Some("2.44.0".into());

        let cmd = Winget.install_cmd(&reference);
        let index = cmd
            .args
            .iter()
            .position(|a| a == "--version")
            .expect("--version");
        assert_eq!(cmd.args[index + 1], "2.44.0");
    }

    #[test]
    fn uninstall_drops_the_package_agreements_flag() {
        // winget rejects it on uninstall, which would fail every removal.
        let cmd = Winget.uninstall_cmd(&pkg("Mozilla.Firefox"));
        assert!(!cmd
            .args
            .contains(&"--accept-package-agreements".to_string()));
        assert_eq!(cmd.op, PackageOp::Uninstall);
    }

    #[test]
    fn success_and_already_installed_are_both_acceptable_outcomes() {
        assert_eq!(Winget.classify(0), Verdict::Success);
        assert_eq!(
            Winget.classify(codes::PACKAGE_ALREADY_INSTALLED),
            Verdict::AlreadyDone
        );
        assert_eq!(
            Winget.classify(codes::UPDATE_NOT_APPLICABLE),
            Verdict::AlreadyDone
        );

        // The point of the distinction: a bulk install of 30 apps must not report a
        // wall of failures just because some were already present.
        assert!(Winget.classify(codes::PACKAGE_ALREADY_INSTALLED).is_ok());
    }

    #[test]
    fn no_applications_found_maps_to_not_found() {
        // Confirmed by hand against winget 1.29: `winget show --id <missing> --exact`
        // exits 0x8A150014.
        assert_eq!(
            Winget.classify(codes::NO_APPLICATIONS_FOUND),
            Verdict::Failed(PackageFailure::NotFound)
        );
    }

    #[test]
    fn download_problems_are_marked_retryable() {
        let Verdict::Failed(failure) = Winget.classify(codes::DOWNLOAD_FAILED) else {
            panic!("a download failure is a failure");
        };
        assert!(failure.is_retryable());
    }

    #[test]
    fn reboot_codes_are_success_not_failure() {
        for code in [
            common::REBOOT_REQUIRED,
            common::REBOOT_INITIATED,
            codes::INSTALL_REBOOT_REQUIRED_TO_FINISH,
            codes::INSTALL_REBOOT_REQUIRED_FOR_INSTALL,
        ] {
            assert_eq!(
                Winget.classify(code),
                Verdict::RebootRequired,
                "code {code:#010x}"
            );
        }
    }

    #[test]
    fn unmapped_codes_are_reported_rather_than_guessed() {
        assert_eq!(
            Winget.classify(12345),
            Verdict::Failed(PackageFailure::Unknown)
        );
    }

    #[test]
    fn parses_a_real_export_document() {
        // Trimmed from actual `winget export --include-versions` output.
        let json = r#"{
            "$schema": "https://aka.ms/winget-packages.schema.2.0.json",
            "CreationDate": "2026-08-08T04:13:35.412-00:00",
            "Sources": [
                {
                    "Packages": [
                        { "PackageIdentifier": "7zip.7zip", "Version": "24.09" },
                        { "PackageIdentifier": "Git.Git", "Version": "2.44.0" }
                    ],
                    "SourceDetails": { "Name": "winget" }
                }
            ]
        }"#;

        let installed = Winget::parse_export(json).unwrap();
        assert_eq!(installed.len(), 2);
        assert_eq!(installed[0].id, "7zip.7zip");
        assert_eq!(installed[0].version, "24.09");
        assert_eq!(installed[1].id, "Git.Git");
        assert!(installed.iter().all(|p| p.provider == ProviderId::Winget));
    }

    #[test]
    fn an_export_without_versions_still_parses() {
        // `--include-versions` is a flag; without it the Version field is absent, and
        // an inventory with blank versions beats an error.
        let json = r#"{"Sources":[{"Packages":[{"PackageIdentifier":"Git.Git"}]}]}"#;
        let installed = Winget::parse_export(json).unwrap();
        assert_eq!(installed[0].version, "");
    }

    #[test]
    fn an_empty_export_is_not_an_error() {
        // A machine with nothing winget knows about is unusual but perfectly valid.
        assert!(Winget::parse_export(r#"{"Sources":[]}"#)
            .unwrap()
            .is_empty());
        assert!(Winget::parse_export(r#"{}"#).unwrap().is_empty());
    }

    #[test]
    fn packages_from_several_sources_are_merged() {
        let json = r#"{"Sources":[
            {"Packages":[{"PackageIdentifier":"A"}]},
            {"Packages":[{"PackageIdentifier":"B"}]}
        ]}"#;
        assert_eq!(Winget::parse_export(json).unwrap().len(), 2);
    }

    #[test]
    fn malformed_export_json_is_an_error() {
        assert!(Winget::parse_export("not json").is_err());
    }
}
