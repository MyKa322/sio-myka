//! The Chocolatey provider.
//!
//! Chocolatey has the broadest catalog, especially for older utilities winget never
//! picked up. It needs bootstrapping — it is not present on a fresh Windows — so it is
//! always a secondary source.
//!
//! Unlike winget it does offer machine-readable output: `--limit-output` prints
//! `name|version` with no headers, decoration or translation. That format is stable and
//! safe to parse, and was confirmed against Chocolatey 2.6.0.

use crate::provider::{common, probe, PackageProvider, Verdict};
use async_trait::async_trait;
use sio_core::error::{PackageFailure, Result};
use sio_core::package::{InstalledPackage, PackageCmd, PackageOp, PackageRef, ProviderId};

#[derive(Debug, Default, Clone, Copy)]
pub struct Chocolatey;

impl Chocolatey {
    pub fn new() -> Self {
        Self
    }

    /// Parse `choco list --limit-output` output.
    ///
    /// One `name|version` per line. Lines that do not match are skipped rather than
    /// failing the whole inventory: Chocolatey occasionally prefixes warnings even
    /// under `--limit-output`, and losing the entire list over one stray line would be
    /// a poor trade.
    pub(crate) fn parse_list(output: &str) -> Vec<InstalledPackage> {
        output
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                let (name, version) = line.split_once('|')?;
                if name.is_empty() || version.contains('|') {
                    return None;
                }
                Some(InstalledPackage {
                    provider: ProviderId::Chocolatey,
                    // Chocolatey ids are case-insensitive; normalise so comparisons
                    // against catalog entries behave.
                    id: name.to_lowercase(),
                    name: name.to_string(),
                    version: version.trim().to_string(),
                    available_version: None,
                })
            })
            .collect()
    }
}

fn base_flags() -> Vec<String> {
    vec![
        // `-y` is essential: without it Chocolatey waits for a confirmation nobody can
        // give, since the process has no console.
        "-y".into(),
        "--no-progress".into(),
        "--limit-output".into(),
    ]
}

#[async_trait]
impl PackageProvider for Chocolatey {
    fn id(&self) -> ProviderId {
        ProviderId::Chocolatey
    }

    async fn is_available(&self) -> bool {
        probe("choco", &["--version"]).await
    }

    async fn installed(&self) -> Result<Vec<InstalledPackage>> {
        // Chocolatey 2.x lists local packages by default; the old `-lo` flag was
        // removed and passing it is now an error.
        let args = vec![
            "list".into(),
            "--limit-output".into(),
            "--no-progress".into(),
        ];
        let (_, stdout) = sio_winsys::process::run_captured("choco", &args).await?;
        Ok(Self::parse_list(&stdout))
    }

    fn install_cmd(&self, pkg: &PackageRef) -> PackageCmd {
        let mut args = vec!["install".into(), pkg.id.clone()];
        args.extend(base_flags());
        if let Some(version) = &pkg.version {
            args.push("--version".into());
            args.push(version.clone());
        }

        PackageCmd {
            provider: ProviderId::Chocolatey,
            op: PackageOp::Install,
            program: "choco".into(),
            args,
            elevated: true,
        }
    }

    fn uninstall_cmd(&self, pkg: &PackageRef) -> PackageCmd {
        let mut args = vec!["uninstall".into(), pkg.id.clone()];
        args.extend(base_flags());

        PackageCmd {
            provider: ProviderId::Chocolatey,
            op: PackageOp::Uninstall,
            program: "choco".into(),
            args,
            elevated: true,
        }
    }

    fn classify(&self, code: i32) -> Verdict {
        match code {
            // Chocolatey returns 0 for an already-installed package too: it warns and
            // skips rather than erroring, so there is no separate code to map.
            0 => Verdict::Success,
            common::REBOOT_REQUIRED | common::REBOOT_INITIATED => Verdict::RebootRequired,
            _ => Verdict::Failed(PackageFailure::InstallFailed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_limit_output_format() {
        // Verbatim from `choco list --limit-output` on Chocolatey 2.6.0.
        let output = "chocolatey|2.6.0\n\
                      chocolatey-core.extension|1.4.0\n\
                      dotnetfx|4.8.0.20220524\n\
                      KB2919355|1.0.20160915\n";

        let installed = Chocolatey::parse_list(output);
        assert_eq!(installed.len(), 4);
        assert_eq!(installed[0].name, "chocolatey");
        assert_eq!(installed[0].version, "2.6.0");
        assert!(installed
            .iter()
            .all(|p| p.provider == ProviderId::Chocolatey));
    }

    #[test]
    fn ids_are_lowercased_so_catalog_matching_works() {
        // The catalog says "kb2919355"; Chocolatey reports "KB2919355".
        let installed = Chocolatey::parse_list("KB2919355|1.0.0\n");
        assert_eq!(installed[0].id, "kb2919355");
        assert_eq!(
            installed[0].name, "KB2919355",
            "the display name keeps its casing"
        );
    }

    #[test]
    fn stray_lines_are_skipped_without_losing_the_rest() {
        let output = "Chocolatey v2.6.0\n\
                      firefox|153.0.3\n\
                      \n\
                      some warning text\n\
                      vlc|3.0.23\n";

        let installed = Chocolatey::parse_list(output);
        assert_eq!(
            installed.len(),
            2,
            "one bad line must not discard the inventory"
        );
        assert_eq!(installed[0].id, "firefox");
        assert_eq!(installed[1].id, "vlc");
    }

    #[test]
    fn empty_output_yields_an_empty_inventory() {
        assert!(Chocolatey::parse_list("").is_empty());
        assert!(Chocolatey::parse_list("\n\n").is_empty());
    }

    #[test]
    fn install_always_passes_the_confirmation_flag() {
        // Without -y, Chocolatey blocks forever on a prompt that has no console.
        let cmd = Chocolatey.install_cmd(&PackageRef::new(ProviderId::Chocolatey, "firefox"));
        assert!(cmd.args.contains(&"-y".to_string()));
        assert!(cmd.elevated);
        assert_eq!(cmd.program, "choco");
    }

    #[test]
    fn a_pinned_version_is_passed_through() {
        let mut reference = PackageRef::new(ProviderId::Chocolatey, "vlc");
        reference.version = Some("3.0.20".into());
        let cmd = Chocolatey.install_cmd(&reference);
        let index = cmd
            .args
            .iter()
            .position(|a| a == "--version")
            .expect("--version");
        assert_eq!(cmd.args[index + 1], "3.0.20");
    }

    #[test]
    fn reboot_codes_are_not_treated_as_failures() {
        assert_eq!(
            Chocolatey.classify(common::REBOOT_REQUIRED),
            Verdict::RebootRequired
        );
        assert_eq!(
            Chocolatey.classify(common::REBOOT_INITIATED),
            Verdict::RebootRequired
        );
        assert_eq!(Chocolatey.classify(0), Verdict::Success);
        assert!(!Chocolatey.classify(1).is_ok());
    }
}
