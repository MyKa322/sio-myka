//! The Scoop provider.
//!
//! Scoop installs into the user profile and deliberately avoids elevation — running it
//! as administrator puts shims in the wrong profile and is a documented mistake. So
//! unlike the other two, its commands run in the app's own (unelevated) process.
//!
//! **Verification status:** Scoop was not installed on the machine this was written on,
//! so unlike winget and Chocolatey the output handling here is written from Scoop's
//! documented formats rather than confirmed against a live install. It is deliberately
//! forgiving: `scoop export` switched from plain text to JSON at v0.3.0, so both are
//! accepted, and an unparseable inventory yields an empty list instead of an error.

use crate::provider::{probe, PackageProvider, Verdict};
use async_trait::async_trait;
use serde::Deserialize;
use sio_core::error::{PackageFailure, Result};
use sio_core::package::{InstalledPackage, PackageCmd, PackageOp, PackageRef, ProviderId};

#[derive(Debug, Default, Clone, Copy)]
pub struct Scoop;

#[derive(Debug, Deserialize)]
struct ExportDocument {
    #[serde(default)]
    apps: Vec<ExportApp>,
}

#[derive(Debug, Deserialize)]
struct ExportApp {
    #[serde(rename = "Name", alias = "name")]
    name: String,
    #[serde(rename = "Version", alias = "version", default)]
    version: String,
}

impl Scoop {
    pub fn new() -> Self {
        Self
    }

    /// Parse `scoop export` output, accepting either the modern JSON form or the older
    /// plain-text listing.
    pub(crate) fn parse_export(output: &str) -> Vec<InstalledPackage> {
        let trimmed = output.trim();
        if trimmed.is_empty() {
            return Vec::new();
        }

        // Anything starting with `{` is meant to be JSON. If it will not parse, give
        // up here rather than falling through to the text parser — that would happily
        // read `{` as a package name and invent an app that does not exist.
        if trimmed.starts_with('{') {
            return match serde_json::from_str::<ExportDocument>(trimmed) {
                Ok(document) => document
                    .apps
                    .into_iter()
                    .map(|app| InstalledPackage {
                        provider: ProviderId::Scoop,
                        id: app.name.to_lowercase(),
                        name: app.name,
                        version: app.version,
                        available_version: None,
                    })
                    .collect(),
                Err(e) => {
                    tracing::warn!("could not read the scoop export: {e}");
                    Vec::new()
                }
            };
        }

        // Legacy form: "name version [bucket]" per line.
        trimmed
            .lines()
            .filter_map(|line| {
                let mut parts = line.split_whitespace();
                let name = parts.next()?;
                // Skip anything that looks like a header or a bare bucket line.
                if name.eq_ignore_ascii_case("name") {
                    return None;
                }
                Some(InstalledPackage {
                    provider: ProviderId::Scoop,
                    id: name.to_lowercase(),
                    name: name.to_string(),
                    version: parts.next().unwrap_or_default().to_string(),
                    available_version: None,
                })
            })
            .collect()
    }
}

#[async_trait]
impl PackageProvider for Scoop {
    fn id(&self) -> ProviderId {
        ProviderId::Scoop
    }

    async fn is_available(&self) -> bool {
        probe("scoop", &["--version"]).await
    }

    async fn installed(&self) -> Result<Vec<InstalledPackage>> {
        let (_, stdout) =
            sio_winsys::process::run_captured("scoop", &["export".to_string()]).await?;
        Ok(Self::parse_export(&stdout))
    }

    fn install_cmd(&self, pkg: &PackageRef) -> PackageCmd {
        let mut args = vec!["install".into()];
        // Scoop pins with `name@version` rather than a flag.
        args.push(match &pkg.version {
            Some(version) => format!("{}@{}", pkg.id, version),
            None => pkg.id.clone(),
        });

        PackageCmd {
            provider: ProviderId::Scoop,
            op: PackageOp::Install,
            program: "scoop".into(),
            args,
            // Never elevated. Scoop installs into the user profile, and running it as
            // administrator puts the shims under the wrong account.
            elevated: false,
        }
    }

    fn uninstall_cmd(&self, pkg: &PackageRef) -> PackageCmd {
        PackageCmd {
            provider: ProviderId::Scoop,
            op: PackageOp::Uninstall,
            program: "scoop".into(),
            args: vec!["uninstall".into(), pkg.id.clone()],
            elevated: false,
        }
    }

    fn classify(&self, code: i32) -> Verdict {
        match code {
            0 => Verdict::Success,
            _ => Verdict::Failed(PackageFailure::InstallFailed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoop_commands_are_never_elevated() {
        // The whole point of Scoop: elevation would install into the wrong profile.
        assert!(
            !Scoop
                .install_cmd(&PackageRef::new(ProviderId::Scoop, "7zip"))
                .elevated
        );
        assert!(
            !Scoop
                .uninstall_cmd(&PackageRef::new(ProviderId::Scoop, "7zip"))
                .elevated
        );
    }

    #[test]
    fn a_pinned_version_uses_at_syntax() {
        let mut reference = PackageRef::new(ProviderId::Scoop, "git");
        reference.version = Some("2.44.0".into());
        let cmd = Scoop.install_cmd(&reference);
        assert!(
            cmd.args.contains(&"git@2.44.0".to_string()),
            "got {:?}",
            cmd.args
        );
    }

    #[test]
    fn parses_the_json_export_form() {
        let json = r#"{
            "buckets": [{ "Name": "main" }],
            "apps": [
                { "Name": "7zip", "Version": "24.09", "Source": "main" },
                { "Name": "Git", "Version": "2.44.0", "Source": "main" }
            ]
        }"#;

        let installed = Scoop::parse_export(json);
        assert_eq!(installed.len(), 2);
        assert_eq!(installed[0].id, "7zip");
        assert_eq!(
            installed[1].id, "git",
            "ids are lowercased for catalog matching"
        );
        assert_eq!(installed[1].version, "2.44.0");
    }

    #[test]
    fn parses_the_legacy_plain_text_form() {
        // scoop export emitted plain text before v0.3.0.
        let text = "7zip 24.09 main\ngit 2.44.0 main\n";
        let installed = Scoop::parse_export(text);
        assert_eq!(installed.len(), 2);
        assert_eq!(installed[0].id, "7zip");
        assert_eq!(installed[0].version, "24.09");
    }

    #[test]
    fn unparseable_output_yields_an_empty_list_rather_than_an_error() {
        // Scoop is the one provider written without a live install to test against, so
        // it fails soft: an empty inventory degrades the UI, an error breaks it.
        assert!(Scoop::parse_export("").is_empty());
        assert!(Scoop::parse_export("   \n  ").is_empty());
        assert!(Scoop::parse_export("{ this is not valid json").is_empty());
    }

    #[test]
    fn a_json_export_with_no_apps_is_empty() {
        assert!(Scoop::parse_export(r#"{"buckets":[],"apps":[]}"#).is_empty());
    }
}
