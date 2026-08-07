//! Tweak definitions and the reversible-action model.
//!
//! Tweaks are *data*, loaded from `catalog/tweaks.json`. Adding a tweak must never
//! require a code change.
//!
//! The reversibility guarantee rests on one rule: before mutating anything we capture
//! its prior state as a [`PriorValue`] / [`PriorState`], which explicitly models
//! "this did not exist". Reverting a value that was previously absent means *deleting*
//! it — writing some plausible-looking default instead is how tools silently corrupt a
//! system, because a policy key set to its "default" is not the same as no policy key.

use crate::text::LocalizedText;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Hive {
    Hklm,
    Hkcu,
    Hkcr,
    Hku,
}

/// A registry value. Mirrors the subset of `REG_*` types the catalog is allowed to use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "value_type", content = "value", rename_all = "snake_case")]
pub enum RegistryValue {
    Dword(u32),
    Qword(u64),
    String(String),
    ExpandString(String),
    MultiString(Vec<String>),
    Binary(Vec<u8>),
}

/// A single registry write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryEdit {
    pub hive: Hive,
    /// Subkey path, without the hive prefix.
    pub path: String,
    /// Value name. Empty string addresses the key's default value.
    pub name: String,
    #[serde(flatten)]
    pub value: RegistryValue,
}

/// What was at a registry location before we touched it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PriorValue {
    /// The value did not exist. Reverting means deleting it again.
    Absent,
    /// Neither the value nor its parent key existed. Reverting deletes the value; the
    /// key itself is left in place, since other tweaks may share it.
    KeyAbsent,
    Present(RegistryValue),
}

// ---------------------------------------------------------------------------
// Services
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceStartType {
    Boot,
    System,
    Automatic,
    Manual,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceConfig {
    pub name: String,
    pub start_type: ServiceStartType,
    /// Also stop the service now, rather than only on next boot.
    #[serde(default)]
    pub stop: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriorState {
    pub name: String,
    pub start_type: ServiceStartType,
    pub was_running: bool,
}

// ---------------------------------------------------------------------------
// Appx / UWP
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppxRef {
    /// Package family name, e.g. `Microsoft.BingNews_8wekyb3d8bbwe`.
    pub package_family_name: String,
    /// Also remove the provisioned copy so new user profiles don't get it back.
    #[serde(default)]
    pub deprovision: bool,
}

// ---------------------------------------------------------------------------
// Tweak definitions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    /// Cosmetic or privacy settings, trivially reversible.
    Low,
    /// Changes system behaviour in a way a user might notice and not expect.
    Medium,
    /// Can break functionality or is awkward to undo. Requires explicit confirmation.
    High,
}

impl Risk {
    /// Anything above `Low` must be individually confirmed, never applied by a
    /// "select all" click.
    pub fn needs_confirmation(self) -> bool {
        self > Risk::Low
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TweakCategory {
    Privacy,
    Telemetry,
    Performance,
    Interface,
    Debloat,
    Gaming,
    Updates,
}

/// One reversible operation. Tweaks are composed of these.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TweakAction {
    Registry(RegistryEdit),
    Service(ServiceConfig),
    Appx(AppxRef),
}

/// Which Windows versions a tweak applies to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Applies {
    #[default]
    Both,
    Windows10Only,
    Windows11Only,
}

impl Applies {
    pub fn matches(self, build: u32) -> bool {
        let is_11 = build >= 22000;
        match self {
            Self::Both => true,
            Self::Windows10Only => !is_11,
            Self::Windows11Only => is_11,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tweak {
    /// Stable dotted identifier, e.g. `privacy.telemetry.disable`. Referenced by
    /// profiles and the journal, so it must never be renamed once released.
    pub id: String,
    /// Display name, translated in the catalog itself — see [`crate::text`].
    pub name: LocalizedText,
    #[serde(default, skip_serializing_if = "LocalizedText::is_empty")]
    pub description: LocalizedText,
    pub category: TweakCategory,
    pub risk: Risk,
    #[serde(default)]
    pub applies: Applies,
    #[serde(default)]
    pub requires_restart: bool,
    pub actions: Vec<TweakAction>,
}

impl Tweak {
    /// Whether every action in this tweak needs administrator rights.
    ///
    /// HKCU writes are the only thing we can do unelevated, so a tweak touching HKLM,
    /// a service, or an Appx package must go through the broker.
    pub fn requires_elevation(&self) -> bool {
        self.actions.iter().any(|a| match a {
            TweakAction::Registry(e) => e.hive != Hive::Hkcu,
            TweakAction::Service(_) | TweakAction::Appx(_) => true,
        })
    }
}

// ---------------------------------------------------------------------------
// Journal
// ---------------------------------------------------------------------------

/// What we captured before performing one action, and therefore how to undo it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AppliedAction {
    Registry {
        hive: Hive,
        path: String,
        name: String,
        prior: PriorValue,
    },
    Service(PriorState),
    /// Appx removal is the one action we cannot undo ourselves: reinstalling requires
    /// the Store. We record it so the UI can tell the user precisely what was removed.
    AppxRemoved {
        package_family_name: String,
        deprovisioned: bool,
    },
}

impl AppliedAction {
    /// Whether this action can be undone by replaying the journal.
    pub fn is_reversible(&self) -> bool {
        !matches!(self, Self::AppxRemoved { .. })
    }
}

/// One tweak application, with everything needed to reverse it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub tweak_id: String,
    /// Unix milliseconds.
    pub applied_at: u64,
    pub actions: Vec<AppliedAction>,
}

impl JournalEntry {
    /// Actions to replay in order to undo this entry.
    ///
    /// Reversed relative to application order: if two actions touched the same value,
    /// the *earliest* capture holds the true original, so it must be restored last.
    pub fn revert_plan(&self) -> Vec<&AppliedAction> {
        self.actions
            .iter()
            .rev()
            .filter(|a| a.is_reversible())
            .collect()
    }

    /// Actions that cannot be undone, for warning the user before they try.
    pub fn irreversible(&self) -> Vec<&AppliedAction> {
        self.actions.iter().filter(|a| !a.is_reversible()).collect()
    }

    pub fn is_fully_reversible(&self) -> bool {
        self.actions.iter().all(AppliedAction::is_reversible)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg_action(name: &str, prior: PriorValue) -> AppliedAction {
        AppliedAction::Registry {
            hive: Hive::Hklm,
            path: "SOFTWARE\\Test".into(),
            name: name.into(),
            prior,
        }
    }

    #[test]
    fn revert_plan_runs_newest_first() {
        // Both actions touched the same value. The first capture (Absent) is the real
        // original state, so it must be applied last to win.
        let entry = JournalEntry {
            tweak_id: "t".into(),
            applied_at: 0,
            actions: vec![
                reg_action("Same", PriorValue::Absent),
                reg_action("Same", PriorValue::Present(RegistryValue::Dword(1))),
            ],
        };

        let plan = entry.revert_plan();
        assert_eq!(plan.len(), 2);
        assert!(
            matches!(
                plan[1],
                AppliedAction::Registry {
                    prior: PriorValue::Absent,
                    ..
                }
            ),
            "the earliest capture must be restored last so it wins"
        );
    }

    #[test]
    fn absent_and_present_are_distinct_states() {
        // The whole safety property depends on these never collapsing into each other.
        assert_ne!(
            PriorValue::Absent,
            PriorValue::Present(RegistryValue::Dword(0))
        );
        assert_ne!(PriorValue::Absent, PriorValue::KeyAbsent);
    }

    #[test]
    fn appx_removal_is_excluded_from_the_revert_plan() {
        let entry = JournalEntry {
            tweak_id: "debloat.news".into(),
            applied_at: 0,
            actions: vec![
                reg_action("Flag", PriorValue::Absent),
                AppliedAction::AppxRemoved {
                    package_family_name: "Microsoft.BingNews_8wekyb3d8bbwe".into(),
                    deprovisioned: true,
                },
            ],
        };

        assert_eq!(
            entry.revert_plan().len(),
            1,
            "appx removal is not replayable"
        );
        assert_eq!(entry.irreversible().len(), 1);
        assert!(!entry.is_fully_reversible());
    }

    #[test]
    fn hkcu_only_tweaks_do_not_need_elevation() {
        let tweak = Tweak {
            id: "interface.taskbar.left".into(),
            name: LocalizedText::from_iter([("en", "Align taskbar left")]),
            description: LocalizedText::new(),
            category: TweakCategory::Interface,
            risk: Risk::Low,
            applies: Applies::Windows11Only,
            requires_restart: false,
            actions: vec![TweakAction::Registry(RegistryEdit {
                hive: Hive::Hkcu,
                path: "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Advanced".into(),
                name: "TaskbarAl".into(),
                value: RegistryValue::Dword(0),
            })],
        };
        assert!(!tweak.requires_elevation());
    }

    #[test]
    fn any_hklm_write_forces_elevation() {
        let tweak = Tweak {
            id: "privacy.telemetry.disable".into(),
            name: LocalizedText::from_iter([("en", "Disable telemetry")]),
            description: LocalizedText::new(),
            category: TweakCategory::Telemetry,
            risk: Risk::Low,
            applies: Applies::Both,
            requires_restart: false,
            actions: vec![
                TweakAction::Registry(RegistryEdit {
                    hive: Hive::Hkcu,
                    path: "Software\\Test".into(),
                    name: "A".into(),
                    value: RegistryValue::Dword(0),
                }),
                TweakAction::Registry(RegistryEdit {
                    hive: Hive::Hklm,
                    path: "SOFTWARE\\Policies".into(),
                    name: "AllowTelemetry".into(),
                    value: RegistryValue::Dword(0),
                }),
            ],
        };
        assert!(tweak.requires_elevation());
    }

    #[test]
    fn applies_gates_on_the_22000_build_boundary() {
        assert!(Applies::Windows11Only.matches(26100));
        assert!(!Applies::Windows11Only.matches(19045));
        assert!(Applies::Windows10Only.matches(19045));
        assert!(!Applies::Windows10Only.matches(22000));
        assert!(Applies::Both.matches(19045) && Applies::Both.matches(26100));
    }

    #[test]
    fn only_low_risk_skips_confirmation() {
        assert!(!Risk::Low.needs_confirmation());
        assert!(Risk::Medium.needs_confirmation());
        assert!(Risk::High.needs_confirmation());
    }

    #[test]
    fn registry_edit_round_trips_through_json() {
        let edit = RegistryEdit {
            hive: Hive::Hklm,
            path: "SOFTWARE\\Policies\\Microsoft\\Windows\\DataCollection".into(),
            name: "AllowTelemetry".into(),
            value: RegistryValue::Dword(0),
        };
        let json = serde_json::to_string(&edit).unwrap();
        // The catalog format flattens value_type/value alongside the other fields.
        assert!(json.contains("\"value_type\":\"dword\""), "got {json}");
        assert_eq!(serde_json::from_str::<RegistryEdit>(&json).unwrap(), edit);
    }
}
