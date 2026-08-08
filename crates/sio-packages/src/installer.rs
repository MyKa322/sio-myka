//! Turning a selection of catalog apps into actual installs.
//!
//! Three rules shape this module:
//!
//! 1. **One failure does not stop the batch.** Installing thirty apps and aborting on
//!    the fourth is the worst possible outcome after a reinstall. Every item is
//!    attempted and the report says what happened to each.
//! 2. **Already-installed apps are skipped before anything runs.** Cheaper than
//!    launching a package manager to be told the same thing, and it means the outcome
//!    does not depend on an exit code we may have mapped wrong.
//! 3. **Elevation is per command, not per batch.** winget and Chocolatey go through the
//!    broker; Scoop must run unelevated, in this process.

use crate::provider::{PackageProvider, Verdict};
use crate::registry::ProviderRegistry;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sio_core::error::Result;
use sio_core::package::PackageCmd;
use sio_core::privileged::PrivilegedOps;
use sio_core::progress::{Outcome, ProgressSink};
use std::sync::Arc;

/// One thing to install: a catalog app id plus the source chosen for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanItem {
    pub app_id: String,
    /// Human-facing name, used in progress messages.
    pub display_name: String,
    pub package: sio_core::package::PackageRef,
}

/// What happened to one item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemReport {
    pub app_id: String,
    pub display_name: String,
    #[serde(flatten)]
    pub outcome: Outcome,
    /// Raw exit code, when a command actually ran. For bug reports.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub reboot_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallReport {
    pub items: Vec<ItemReport>,
}

impl InstallReport {
    pub fn succeeded(&self) -> usize {
        self.items.iter().filter(|i| i.outcome.is_success()).count()
    }

    pub fn failed(&self) -> usize {
        self.items.len() - self.succeeded()
    }

    /// Whether any item asked for a reboot to finish.
    pub fn reboot_required(&self) -> bool {
        self.items.iter().any(|i| i.reboot_required)
    }
}

/// Runs a resolved command somewhere appropriate.
///
/// An abstraction so the orchestration can be tested without installing anything.
#[async_trait]
pub trait CommandRunner: Send + Sync {
    async fn run(&self, cmd: &PackageCmd, progress: ProgressSink) -> Result<i32>;
}

/// The production runner: elevated commands go to the broker, everything else runs
/// here.
pub struct RoutingRunner {
    privileged: Arc<dyn PrivilegedOps>,
}

impl RoutingRunner {
    pub fn new(privileged: Arc<dyn PrivilegedOps>) -> Self {
        Self { privileged }
    }
}

impl std::fmt::Debug for RoutingRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoutingRunner").finish_non_exhaustive()
    }
}

#[async_trait]
impl CommandRunner for RoutingRunner {
    async fn run(&self, cmd: &PackageCmd, progress: ProgressSink) -> Result<i32> {
        if cmd.elevated {
            self.privileged.run_package_cmd(cmd, progress).await
        } else {
            // Scoop, and anything else that must stay in the user's own context.
            sio_winsys::process::run_streaming(&cmd.program, &cmd.args, &progress).await
        }
    }
}

/// Install everything in `items`, reporting progress as it goes.
pub async fn install_all(
    items: &[PlanItem],
    providers: &ProviderRegistry,
    runner: &dyn CommandRunner,
    progress: ProgressSink,
) -> InstallReport {
    // Snapshot what is already present, taken once at detection rather than per item.
    let already = providers.installed_ids();

    let mut reports = Vec::with_capacity(items.len());

    for item in items {
        progress.started(&item.display_name);

        let Some(provider) = providers.get(item.package.provider) else {
            reports.push(skipped_report(
                item,
                format!("{} is not available on this system", item.package.provider),
            ));
            continue;
        };

        if already.contains(&(item.package.provider, item.package.id.to_lowercase())) {
            let report = ItemReport {
                app_id: item.app_id.clone(),
                display_name: item.display_name.clone(),
                outcome: Outcome::Skipped {
                    reason: "already installed".into(),
                },
                exit_code: None,
                reboot_required: false,
            };
            progress.finished(&item.display_name, report.outcome.clone());
            reports.push(report);
            continue;
        }

        let report = install_one(item, provider, runner, &progress).await;
        progress.finished(&item.display_name, report.outcome.clone());
        reports.push(report);
    }

    InstallReport { items: reports }
}

fn skipped_report(item: &PlanItem, reason: String) -> ItemReport {
    ItemReport {
        app_id: item.app_id.clone(),
        display_name: item.display_name.clone(),
        outcome: Outcome::Skipped { reason },
        exit_code: None,
        reboot_required: false,
    }
}

async fn install_one(
    item: &PlanItem,
    provider: &dyn PackageProvider,
    runner: &dyn CommandRunner,
    progress: &ProgressSink,
) -> ItemReport {
    let cmd = provider.install_cmd(&item.package);

    match runner.run(&cmd, progress.clone()).await {
        Ok(code) => {
            let verdict = provider.classify(code);
            let (outcome, reboot) = match verdict {
                Verdict::Success => (Outcome::Success, false),
                Verdict::RebootRequired => (Outcome::Success, true),
                Verdict::AlreadyDone => (
                    Outcome::Skipped {
                        reason: "already installed".into(),
                    },
                    false,
                ),
                Verdict::Failed(failure) => (
                    Outcome::Failed {
                        message: format!("{failure:?} (exit {code:#010x})"),
                    },
                    false,
                ),
            };

            ItemReport {
                app_id: item.app_id.clone(),
                display_name: item.display_name.clone(),
                outcome,
                exit_code: Some(code),
                reboot_required: reboot,
            }
        }
        // The command could not even be started — a missing package manager, or a
        // broker that went away. Distinct from an install that ran and failed.
        Err(e) => ItemReport {
            app_id: item.app_id.clone(),
            display_name: item.display_name.clone(),
            outcome: Outcome::Failed {
                message: e.to_string(),
            },
            exit_code: None,
            reboot_required: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::winget::Winget;
    use sio_core::package::{PackageRef, ProviderId};
    use std::sync::Mutex;

    /// Records what it was asked to run and replies with canned exit codes.
    #[derive(Default)]
    struct FakeRunner {
        codes: Mutex<Vec<i32>>,
        seen: Mutex<Vec<PackageCmd>>,
    }

    impl FakeRunner {
        fn with_codes(codes: &[i32]) -> Self {
            Self {
                codes: Mutex::new(codes.iter().rev().copied().collect()),
                seen: Mutex::new(Vec::new()),
            }
        }
        fn ran(&self) -> Vec<PackageCmd> {
            self.seen.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl CommandRunner for FakeRunner {
        async fn run(&self, cmd: &PackageCmd, _progress: ProgressSink) -> Result<i32> {
            self.seen.lock().unwrap().push(cmd.clone());
            Ok(self.codes.lock().unwrap().pop().unwrap_or(0))
        }
    }

    fn item(id: &str) -> PlanItem {
        PlanItem {
            app_id: id.into(),
            display_name: id.into(),
            package: PackageRef::new(ProviderId::Winget, format!("Pub.{id}")),
        }
    }

    /// A registry with winget present but reporting nothing installed.
    fn registry_with_nothing_installed() -> ProviderRegistry {
        ProviderRegistry::from_parts(
            vec![Box::new(Winget::new())],
            vec![ProviderId::Winget],
            vec![],
        )
    }

    #[tokio::test]
    async fn every_item_is_attempted_even_after_a_failure() {
        // The behaviour that matters most after a reinstall: one bad package must not
        // abandon the other twenty-nine.
        let runner = FakeRunner::with_codes(&[0, 0x8A15_0014u32 as i32, 0]);
        let items = [item("a"), item("b"), item("c")];

        let report = install_all(
            &items,
            &registry_with_nothing_installed(),
            &runner,
            ProgressSink::null(),
        )
        .await;

        assert_eq!(runner.ran().len(), 3, "all three must be attempted");
        assert_eq!(report.succeeded(), 2);
        assert_eq!(report.failed(), 1);
        assert_eq!(report.items[1].app_id, "b");
    }

    #[tokio::test]
    async fn already_installed_apps_are_skipped_without_running_anything() {
        let registry = ProviderRegistry::from_parts(
            vec![Box::new(Winget::new())],
            vec![ProviderId::Winget],
            vec![(ProviderId::Winget, "pub.a".to_string())],
        );

        let runner = FakeRunner::default();
        let report = install_all(
            &[item("a"), item("b")],
            &registry,
            &runner,
            ProgressSink::null(),
        )
        .await;

        assert_eq!(runner.ran().len(), 1, "only the missing one should run");
        assert!(
            runner.ran()[0].args.iter().any(|a| a == "Pub.b"),
            "the one that ran must be the missing app, not the installed one"
        );
        assert!(matches!(report.items[0].outcome, Outcome::Skipped { .. }));
        assert_eq!(report.succeeded(), 2, "a skip counts as success");
    }

    #[tokio::test]
    async fn an_already_installed_exit_code_is_not_a_failure() {
        let runner = FakeRunner::with_codes(&[0x8A15_0061u32 as i32]);
        let report = install_all(
            &[item("a")],
            &registry_with_nothing_installed(),
            &runner,
            ProgressSink::null(),
        )
        .await;

        assert_eq!(report.failed(), 0);
        assert!(matches!(report.items[0].outcome, Outcome::Skipped { .. }));
    }

    #[tokio::test]
    async fn a_reboot_code_counts_as_success_and_is_surfaced() {
        let runner = FakeRunner::with_codes(&[3010]);
        let report = install_all(
            &[item("a")],
            &registry_with_nothing_installed(),
            &runner,
            ProgressSink::null(),
        )
        .await;

        assert_eq!(report.failed(), 0);
        assert!(
            report.reboot_required(),
            "the user needs telling a reboot is pending"
        );
    }

    #[tokio::test]
    async fn an_unavailable_provider_skips_rather_than_erroring() {
        // Scoop-only app on a machine without Scoop.
        let registry = ProviderRegistry::from_parts(
            vec![Box::new(Winget::new())],
            vec![ProviderId::Winget],
            vec![],
        );
        let mut scoop_item = item("a");
        scoop_item.package = PackageRef::new(ProviderId::Scoop, "thing");

        let runner = FakeRunner::default();
        let report = install_all(&[scoop_item], &registry, &runner, ProgressSink::null()).await;

        assert!(runner.ran().is_empty());
        assert!(matches!(report.items[0].outcome, Outcome::Skipped { .. }));
    }

    #[tokio::test]
    async fn progress_is_reported_for_every_item() {
        let (sink, mut rx) = ProgressSink::new();
        let runner = FakeRunner::with_codes(&[0, 0]);
        install_all(
            &[item("a"), item("b")],
            &registry_with_nothing_installed(),
            &runner,
            sink,
        )
        .await;

        let mut started = 0;
        let mut finished = 0;
        while let Ok(progress) = rx.try_recv() {
            match progress {
                sio_core::progress::Progress::Started { .. } => started += 1,
                sio_core::progress::Progress::Finished { .. } => finished += 1,
                _ => {}
            }
        }
        assert_eq!(started, 2);
        assert_eq!(finished, 2);
    }

    #[tokio::test]
    async fn a_runner_error_is_distinct_from_a_failed_install() {
        struct Broken;
        #[async_trait]
        impl CommandRunner for Broken {
            async fn run(&self, _c: &PackageCmd, _p: ProgressSink) -> Result<i32> {
                Err(sio_core::Error::Broker {
                    reason: "helper died".into(),
                })
            }
        }

        let report = install_all(
            &[item("a")],
            &registry_with_nothing_installed(),
            &Broken,
            ProgressSink::null(),
        )
        .await;

        assert_eq!(report.failed(), 1);
        assert_eq!(
            report.items[0].exit_code, None,
            "nothing ran, so there is no exit code"
        );
        let Outcome::Failed { message } = &report.items[0].outcome else {
            panic!("expected a failure");
        };
        assert!(message.contains("helper died"), "got {message}");
    }
}
