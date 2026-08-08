//! Applying and reverting tweaks.
//!
//! The order inside [`apply`] is the whole safety story: for each action the prior
//! state is captured, then the change is made, then the capture is journalled — and the
//! journal is written **even when a later action fails**. A tweak that dies halfway
//! must still be fully undoable, which means recording what was already done rather
//! than discarding it because the operation "failed".

use crate::journal::JournalStore;
use serde::{Deserialize, Serialize};
use sio_core::error::Result;
use sio_core::privileged::{PrivilegedOps, RestorePointOutcome};
use sio_core::progress::{Outcome, ProgressSink};
use sio_core::reader::SystemReader;
use sio_core::tweak::{AppliedAction, JournalEntry, Tweak, TweakAction};

/// Whether a tweak's desired state currently holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TweakStatus {
    Applied,
    NotApplied,
    /// Some actions match and some do not — usually a half-finished apply, or a
    /// Windows update that reset one of several values.
    Partial,
    /// The system could not be read. Never guessed at, because showing a tweak as "off"
    /// when we simply could not look invites the user to apply it twice.
    Unknown,
}

/// Read the machine and decide whether a tweak is in effect.
///
/// Needs no elevation, so the Tuning screen can show accurate state on open.
pub async fn status(reader: &dyn SystemReader, tweak: &Tweak) -> TweakStatus {
    let mut applied = 0usize;
    let mut checked = 0usize;

    for action in &tweak.actions {
        match action_is_applied(reader, action).await {
            Ok(true) => {
                applied += 1;
                checked += 1;
            }
            Ok(false) => checked += 1,
            // One unreadable action makes the whole answer untrustworthy.
            Err(_) => return TweakStatus::Unknown,
        }
    }

    if checked == 0 {
        return TweakStatus::Unknown;
    }
    if applied == checked {
        TweakStatus::Applied
    } else if applied == 0 {
        TweakStatus::NotApplied
    } else {
        TweakStatus::Partial
    }
}

async fn action_is_applied(reader: &dyn SystemReader, action: &TweakAction) -> Result<bool> {
    match action {
        TweakAction::Registry(edit) => {
            let current = reader
                .registry_read(edit.hive, &edit.path, &edit.name)
                .await?;
            Ok(matches!(&current, sio_core::tweak::PriorValue::Present(v) if v == &edit.value))
        }
        TweakAction::Service(cfg) => {
            let state = reader.service_state(&cfg.name).await?;
            let start_matches = state.start_type == cfg.start_type;
            // If the tweak also stops the service, it is only fully applied once the
            // service is actually stopped.
            Ok(start_matches && (!cfg.stop || !state.was_running))
        }
        // Removal is the change, so "applied" means the package is gone.
        TweakAction::Appx(pkg) => Ok(!reader.appx_present(&pkg.package_family_name).await?),
    }
}

/// Perform one action, returning what was there before.
async fn perform(ops: &dyn PrivilegedOps, action: &TweakAction) -> Result<AppliedAction> {
    match action {
        TweakAction::Registry(edit) => {
            let prior = ops.registry_set(edit).await?;
            Ok(AppliedAction::Registry {
                hive: edit.hive,
                path: edit.path.clone(),
                name: edit.name.clone(),
                prior,
            })
        }
        TweakAction::Service(cfg) => {
            let prior = ops.service_configure(cfg).await?;
            Ok(AppliedAction::Service(prior))
        }
        TweakAction::Appx(pkg) => {
            ops.appx_remove(pkg).await?;
            Ok(AppliedAction::AppxRemoved {
                package_family_name: pkg.package_family_name.clone(),
                deprovisioned: pkg.deprovision,
            })
        }
    }
}

/// Apply one tweak, journalling whatever was actually done.
///
/// On failure the error is returned *and* the partial journal entry is written, so the
/// caller can still offer to undo the half that succeeded.
pub async fn apply(
    ops: &dyn PrivilegedOps,
    journal: &JournalStore,
    tweak: &Tweak,
    now: u64,
) -> Result<JournalEntry> {
    let mut actions = Vec::with_capacity(tweak.actions.len());
    let mut failure = None;

    for action in &tweak.actions {
        match perform(ops, action).await {
            Ok(applied) => actions.push(applied),
            Err(e) => {
                failure = Some(e);
                break;
            }
        }
    }

    let entry = JournalEntry::new(&tweak.id, now, actions);

    // Written before the error is returned. Losing the record of a partial apply would
    // leave changes on the machine that nothing knows how to undo.
    if !entry.actions.is_empty() {
        journal.write(&entry).await?;
    }

    match failure {
        Some(e) => Err(e),
        None => Ok(entry),
    }
}

/// Undo a journal entry and mark it reverted.
///
/// Reverting is best-effort per action: if one restore fails the rest are still
/// attempted, because leaving five of six settings changed is worse than reporting one
/// failure.
pub async fn revert(
    ops: &dyn PrivilegedOps,
    journal: &JournalStore,
    entry: &JournalEntry,
    now: u64,
) -> Result<RevertReport> {
    let mut failures = Vec::new();

    for action in entry.revert_plan() {
        let result = match action {
            AppliedAction::Registry {
                hive,
                path,
                name,
                prior,
            } => ops.registry_restore(*hive, path, name, prior).await,
            AppliedAction::Service(prior) => ops.service_restore(prior).await,
            // Filtered out by revert_plan, but matched exhaustively so a future
            // irreversible action cannot be silently skipped here.
            AppliedAction::AppxRemoved { .. } => Ok(()),
        };

        if let Err(e) = result {
            failures.push(e.to_string());
        }
    }

    let mut updated = entry.clone();
    updated.reverted_at = Some(now);
    journal.update(&updated).await?;

    Ok(RevertReport {
        tweak_id: entry.tweak_id.clone(),
        failures,
        irreversible: entry
            .irreversible()
            .iter()
            .map(|a| match a {
                AppliedAction::AppxRemoved {
                    package_family_name,
                    ..
                } => package_family_name.clone(),
                _ => String::new(),
            })
            .collect(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevertReport {
    pub tweak_id: String,
    /// Actions that could not be put back.
    pub failures: Vec<String>,
    /// Things that cannot be undone at all, so the user can be told plainly.
    pub irreversible: Vec<String>,
}

impl RevertReport {
    pub fn is_clean(&self) -> bool {
        self.failures.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TweakOutcome {
    pub tweak_id: String,
    #[serde(flatten)]
    pub outcome: Outcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyReport {
    pub items: Vec<TweakOutcome>,
    /// What happened when we asked for a restore point before starting.
    pub restore_point: Option<RestorePointOutcome>,
    pub restart_required: bool,
}

impl ApplyReport {
    pub fn succeeded(&self) -> usize {
        self.items.iter().filter(|i| i.outcome.is_success()).count()
    }

    pub fn failed(&self) -> usize {
        self.items.len() - self.succeeded()
    }
}

/// Apply a batch, creating a restore point first.
///
/// The restore point outcome is returned rather than swallowed: Windows throttles them
/// to roughly one per day and does nothing at all when System Protection is off, so
/// claiming a safety net that does not exist would be worse than not offering one.
pub async fn apply_all(
    ops: &dyn PrivilegedOps,
    journal: &JournalStore,
    tweaks: &[Tweak],
    now: u64,
    progress: ProgressSink,
) -> ApplyReport {
    let restore_point = if tweaks.is_empty() {
        None
    } else {
        match ops.create_restore_point("Before SIO tuning").await {
            Ok(outcome) => Some(outcome),
            Err(e) => {
                // Not fatal: the user asked for the tweaks, not for the restore point.
                progress.log(format!("could not create a restore point: {e}"));
                None
            }
        }
    };

    let mut items = Vec::with_capacity(tweaks.len());
    let mut restart_required = false;

    for tweak in tweaks {
        progress.started(&tweak.id);

        let outcome = match apply(ops, journal, tweak, now).await {
            Ok(_) => {
                restart_required |= tweak.requires_restart;
                Outcome::Success
            }
            Err(e) => Outcome::Failed {
                message: e.to_string(),
            },
        };

        progress.finished(&tweak.id, outcome.clone());
        items.push(TweakOutcome {
            tweak_id: tweak.id.clone(),
            outcome,
        });
    }

    ApplyReport {
        items,
        restore_point,
        restart_required,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use sio_core::package::PackageCmd;
    use sio_core::text::LocalizedText;
    use sio_core::tweak::{
        Applies, AppxRef, Hive, PriorState, PriorValue, RegistryEdit, RegistryValue, Risk,
        ServiceConfig, ServiceStartType, TweakCategory,
    };
    use std::collections::HashMap;
    use std::sync::Mutex;

    fn store(tag: &str) -> JournalStore {
        let dir = std::env::temp_dir()
            .join("sio-engine-tests")
            .join(format!("{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        JournalStore::new(dir)
    }

    fn reg_edit(name: &str, value: u32) -> RegistryEdit {
        RegistryEdit {
            hive: Hive::Hkcu,
            path: r"Software\SioTest".into(),
            name: name.into(),
            value: RegistryValue::Dword(value),
        }
    }

    fn tweak(id: &str, actions: Vec<TweakAction>) -> Tweak {
        Tweak {
            id: id.into(),
            name: LocalizedText::from_iter([("en", id)]),
            description: LocalizedText::new(),
            category: TweakCategory::Privacy,
            risk: Risk::Low,
            applies: Applies::Both,
            requires_restart: false,
            actions,
        }
    }

    /// An in-memory machine: registry values, service states and installed packages.
    #[derive(Default)]
    struct FakeMachine {
        registry: Mutex<HashMap<String, RegistryValue>>,
        services: Mutex<HashMap<String, PriorState>>,
        packages: Mutex<Vec<String>>,
        /// Names whose writes should fail, to simulate a mid-tweak error.
        fail_on: Mutex<Vec<String>>,
        restore_point: Mutex<Option<RestorePointOutcome>>,
    }

    impl FakeMachine {
        fn key(hive: Hive, path: &str, name: &str) -> String {
            format!("{hive:?}|{path}|{name}")
        }
    }

    #[async_trait]
    impl SystemReader for FakeMachine {
        async fn registry_read(&self, hive: Hive, path: &str, name: &str) -> Result<PriorValue> {
            Ok(
                match self
                    .registry
                    .lock()
                    .unwrap()
                    .get(&Self::key(hive, path, name))
                {
                    Some(v) => PriorValue::Present(v.clone()),
                    None => PriorValue::Absent,
                },
            )
        }
        async fn service_state(&self, name: &str) -> Result<PriorState> {
            self.services
                .lock()
                .unwrap()
                .get(name)
                .cloned()
                .ok_or_else(|| sio_core::Error::Other(format!("no such service `{name}`")))
        }
        async fn appx_present(&self, pfn: &str) -> Result<bool> {
            Ok(self.packages.lock().unwrap().iter().any(|p| p == pfn))
        }
    }

    #[async_trait]
    impl PrivilegedOps for FakeMachine {
        async fn registry_set(&self, edit: &RegistryEdit) -> Result<PriorValue> {
            if self.fail_on.lock().unwrap().contains(&edit.name) {
                return Err(sio_core::Error::Registry {
                    path: edit.path.clone(),
                    reason: "denied".into(),
                });
            }
            let key = Self::key(edit.hive, &edit.path, &edit.name);
            let mut registry = self.registry.lock().unwrap();
            let prior = match registry.get(&key) {
                Some(v) => PriorValue::Present(v.clone()),
                None => PriorValue::Absent,
            };
            registry.insert(key, edit.value.clone());
            Ok(prior)
        }

        async fn registry_restore(
            &self,
            hive: Hive,
            path: &str,
            name: &str,
            prior: &PriorValue,
        ) -> Result<()> {
            let key = Self::key(hive, path, name);
            let mut registry = self.registry.lock().unwrap();
            match prior {
                PriorValue::Present(v) => {
                    registry.insert(key, v.clone());
                }
                PriorValue::Absent | PriorValue::KeyAbsent => {
                    registry.remove(&key);
                }
            }
            Ok(())
        }

        async fn service_configure(&self, cfg: &ServiceConfig) -> Result<PriorState> {
            let mut services = self.services.lock().unwrap();
            let prior = services
                .get(&cfg.name)
                .cloned()
                .ok_or_else(|| sio_core::Error::Other("no such service".into()))?;
            services.insert(
                cfg.name.clone(),
                PriorState {
                    name: cfg.name.clone(),
                    start_type: cfg.start_type,
                    was_running: prior.was_running && !cfg.stop,
                },
            );
            Ok(prior)
        }

        async fn service_restore(&self, prior: &PriorState) -> Result<()> {
            self.services
                .lock()
                .unwrap()
                .insert(prior.name.clone(), prior.clone());
            Ok(())
        }

        async fn appx_remove(&self, pkg: &AppxRef) -> Result<()> {
            self.packages
                .lock()
                .unwrap()
                .retain(|p| p != &pkg.package_family_name);
            Ok(())
        }

        async fn create_restore_point(&self, _d: &str) -> Result<RestorePointOutcome> {
            let outcome = self
                .restore_point
                .lock()
                .unwrap()
                .clone()
                .unwrap_or(RestorePointOutcome::Created { sequence_number: 1 });
            Ok(outcome)
        }

        async fn run_package_cmd(&self, _c: &PackageCmd, _p: ProgressSink) -> Result<i32> {
            Ok(0)
        }
    }

    // --- status ------------------------------------------------------------

    #[tokio::test]
    async fn status_is_not_applied_when_nothing_is_set() {
        let machine = FakeMachine::default();
        let t = tweak("t", vec![TweakAction::Registry(reg_edit("A", 0))]);
        assert_eq!(status(&machine, &t).await, TweakStatus::NotApplied);
    }

    #[tokio::test]
    async fn status_is_applied_only_when_the_value_matches_exactly() {
        let machine = FakeMachine::default();
        let t = tweak("t", vec![TweakAction::Registry(reg_edit("A", 0))]);

        // Present but different — the tweak is not in effect.
        machine.registry.lock().unwrap().insert(
            FakeMachine::key(Hive::Hkcu, r"Software\SioTest", "A"),
            RegistryValue::Dword(1),
        );
        assert_eq!(status(&machine, &t).await, TweakStatus::NotApplied);

        machine.registry.lock().unwrap().insert(
            FakeMachine::key(Hive::Hkcu, r"Software\SioTest", "A"),
            RegistryValue::Dword(0),
        );
        assert_eq!(status(&machine, &t).await, TweakStatus::Applied);
    }

    #[tokio::test]
    async fn a_half_applied_tweak_reports_partial() {
        // The case that matters after a Windows update resets one of several values.
        let machine = FakeMachine::default();
        let t = tweak(
            "t",
            vec![
                TweakAction::Registry(reg_edit("A", 0)),
                TweakAction::Registry(reg_edit("B", 0)),
            ],
        );

        machine.registry.lock().unwrap().insert(
            FakeMachine::key(Hive::Hkcu, r"Software\SioTest", "A"),
            RegistryValue::Dword(0),
        );

        assert_eq!(status(&machine, &t).await, TweakStatus::Partial);
    }

    #[tokio::test]
    async fn an_unreadable_action_makes_the_status_unknown_rather_than_off() {
        // Showing "off" because we could not look would invite applying it twice.
        let machine = FakeMachine::default();
        let t = tweak(
            "t",
            vec![TweakAction::Service(ServiceConfig {
                name: "NoSuchService".into(),
                start_type: ServiceStartType::Disabled,
                stop: true,
            })],
        );
        assert_eq!(status(&machine, &t).await, TweakStatus::Unknown);
    }

    #[tokio::test]
    async fn an_appx_tweak_is_applied_once_the_package_is_gone() {
        let machine = FakeMachine::default();
        machine
            .packages
            .lock()
            .unwrap()
            .push("Microsoft.BingNews_8wekyb3d8bbwe".into());

        let t = tweak(
            "debloat.news",
            vec![TweakAction::Appx(AppxRef {
                package_family_name: "Microsoft.BingNews_8wekyb3d8bbwe".into(),
                deprovision: false,
            })],
        );

        assert_eq!(status(&machine, &t).await, TweakStatus::NotApplied);
        machine.packages.lock().unwrap().clear();
        assert_eq!(status(&machine, &t).await, TweakStatus::Applied);
    }

    // --- apply / revert ----------------------------------------------------

    #[tokio::test]
    async fn apply_then_revert_restores_the_original_value() {
        let machine = FakeMachine::default();
        let journal = store("roundtrip");
        machine.registry.lock().unwrap().insert(
            FakeMachine::key(Hive::Hkcu, r"Software\SioTest", "A"),
            RegistryValue::Dword(7),
        );

        let t = tweak("t", vec![TweakAction::Registry(reg_edit("A", 0))]);
        let entry = apply(&machine, &journal, &t, 1000).await.unwrap();
        assert_eq!(status(&machine, &t).await, TweakStatus::Applied);

        revert(&machine, &journal, &entry, 2000).await.unwrap();

        let restored = machine
            .registry_read(Hive::Hkcu, r"Software\SioTest", "A")
            .await
            .unwrap();
        assert_eq!(restored, PriorValue::Present(RegistryValue::Dword(7)));
    }

    #[tokio::test]
    async fn reverting_a_value_that_did_not_exist_deletes_it() {
        // The property the whole PriorValue model exists for.
        let machine = FakeMachine::default();
        let journal = store("absent");

        let t = tweak("t", vec![TweakAction::Registry(reg_edit("New", 1))]);
        let entry = apply(&machine, &journal, &t, 1000).await.unwrap();
        revert(&machine, &journal, &entry, 2000).await.unwrap();

        let after = machine
            .registry_read(Hive::Hkcu, r"Software\SioTest", "New")
            .await
            .unwrap();
        assert_eq!(
            after,
            PriorValue::Absent,
            "it must be deleted, not set to a default"
        );
    }

    #[tokio::test]
    async fn a_partly_failed_apply_is_still_journalled_and_undoable() {
        // The most important property here: an error must not lose the record of what
        // was already changed, or those changes become permanent by accident.
        let machine = FakeMachine::default();
        let journal = store("partial");
        machine.fail_on.lock().unwrap().push("B".into());

        let t = tweak(
            "t",
            vec![
                TweakAction::Registry(reg_edit("A", 1)),
                TweakAction::Registry(reg_edit("B", 1)),
                TweakAction::Registry(reg_edit("C", 1)),
            ],
        );

        let error = apply(&machine, &journal, &t, 1000).await.unwrap_err();
        assert!(error.to_string().contains("denied"), "got {error}");

        let entries = journal.list().await.unwrap();
        assert_eq!(entries.len(), 1, "the partial apply must still be recorded");
        assert_eq!(
            entries[0].actions.len(),
            1,
            "only the action that succeeded"
        );

        // And it must undo cleanly.
        revert(&machine, &journal, &entries[0], 2000).await.unwrap();
        let after = machine
            .registry_read(Hive::Hkcu, r"Software\SioTest", "A")
            .await
            .unwrap();
        assert_eq!(after, PriorValue::Absent);
    }

    #[tokio::test]
    async fn a_service_tweak_round_trips() {
        let machine = FakeMachine::default();
        let journal = store("service");
        machine.services.lock().unwrap().insert(
            "DiagTrack".into(),
            PriorState {
                name: "DiagTrack".into(),
                start_type: ServiceStartType::Automatic,
                was_running: true,
            },
        );

        let t = tweak(
            "telemetry",
            vec![TweakAction::Service(ServiceConfig {
                name: "DiagTrack".into(),
                start_type: ServiceStartType::Disabled,
                stop: true,
            })],
        );

        let entry = apply(&machine, &journal, &t, 1000).await.unwrap();
        assert_eq!(status(&machine, &t).await, TweakStatus::Applied);

        revert(&machine, &journal, &entry, 2000).await.unwrap();
        let restored = machine.service_state("DiagTrack").await.unwrap();
        assert_eq!(restored.start_type, ServiceStartType::Automatic);
        assert!(restored.was_running);
    }

    #[tokio::test]
    async fn reverting_marks_the_entry_and_reports_irreversible_actions() {
        let machine = FakeMachine::default();
        let journal = store("irreversible");
        machine
            .packages
            .lock()
            .unwrap()
            .push("Microsoft.BingNews_8wekyb3d8bbwe".into());

        let t = tweak(
            "debloat.news",
            vec![
                TweakAction::Registry(reg_edit("A", 1)),
                TweakAction::Appx(AppxRef {
                    package_family_name: "Microsoft.BingNews_8wekyb3d8bbwe".into(),
                    deprovision: true,
                }),
            ],
        );

        let entry = apply(&machine, &journal, &t, 1000).await.unwrap();
        let report = revert(&machine, &journal, &entry, 2000).await.unwrap();

        assert!(report.is_clean());
        assert_eq!(
            report.irreversible,
            vec!["Microsoft.BingNews_8wekyb3d8bbwe"]
        );
        assert!(journal.list().await.unwrap()[0].is_reverted());

        // The registry half came back even though the package cannot.
        let after = machine
            .registry_read(Hive::Hkcu, r"Software\SioTest", "A")
            .await
            .unwrap();
        assert_eq!(after, PriorValue::Absent);
    }

    #[tokio::test]
    async fn apply_all_creates_a_restore_point_and_reports_a_skip_honestly() {
        let machine = FakeMachine::default();
        let journal = store("restore");
        *machine.restore_point.lock().unwrap() = Some(RestorePointOutcome::SkippedThrottled);

        let tweaks = vec![tweak("t", vec![TweakAction::Registry(reg_edit("A", 1))])];
        let report = apply_all(&machine, &journal, &tweaks, 1000, ProgressSink::null()).await;

        assert_eq!(report.succeeded(), 1);
        assert_eq!(
            report.restore_point,
            Some(RestorePointOutcome::SkippedThrottled)
        );
        assert!(
            !report.restore_point.as_ref().unwrap().is_protected(),
            "a throttled restore point is not a safety net and must not claim to be"
        );
    }

    #[tokio::test]
    async fn apply_all_continues_past_a_failing_tweak() {
        let machine = FakeMachine::default();
        let journal = store("continue");
        machine.fail_on.lock().unwrap().push("B".into());

        let tweaks = vec![
            tweak("first", vec![TweakAction::Registry(reg_edit("A", 1))]),
            tweak("second", vec![TweakAction::Registry(reg_edit("B", 1))]),
            tweak("third", vec![TweakAction::Registry(reg_edit("C", 1))]),
        ];

        let report = apply_all(&machine, &journal, &tweaks, 1000, ProgressSink::null()).await;
        assert_eq!(report.items.len(), 3);
        assert_eq!(report.succeeded(), 2);
        assert_eq!(report.failed(), 1);
    }

    #[tokio::test]
    async fn apply_all_skips_the_restore_point_when_there_is_nothing_to_do() {
        let machine = FakeMachine::default();
        let journal = store("empty");
        let report = apply_all(&machine, &journal, &[], 1000, ProgressSink::null()).await;
        assert!(report.restore_point.is_none());
        assert!(report.items.is_empty());
    }

    #[tokio::test]
    async fn restart_required_propagates_from_the_tweak_definition() {
        let machine = FakeMachine::default();
        let journal = store("restart");
        let mut t = tweak("t", vec![TweakAction::Registry(reg_edit("A", 1))]);
        t.requires_restart = true;

        let report = apply_all(&machine, &journal, &[t], 1000, ProgressSink::null()).await;
        assert!(report.restart_required);
    }
}
