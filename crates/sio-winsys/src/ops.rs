//! The in-process implementation of [`PrivilegedOps`].
//!
//! Calls Windows directly. Used in two places: inside the elevated broker, where it does
//! the real work, and in the main process when the app was itself started elevated and
//! a broker would be pointless.
//!
//! Registry and service calls are synchronous Win32 and can block for seconds — waiting
//! for a service to stop, most obviously. They run on the blocking pool so a slow
//! operation cannot stall the async runtime that is streaming progress to the UI.

use crate::{process, registry, services, system};
use async_trait::async_trait;
use sio_core::error::Result;
use sio_core::package::PackageCmd;
use sio_core::privileged::{PrivilegedOps, RestorePointOutcome};
use sio_core::progress::ProgressSink;
use sio_core::tweak::{AppxRef, Hive, PriorState, PriorValue, RegistryEdit, ServiceConfig};

#[derive(Debug, Default, Clone, Copy)]
pub struct InProcessOps;

impl InProcessOps {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl PrivilegedOps for InProcessOps {
    async fn registry_set(&self, edit: &RegistryEdit) -> Result<PriorValue> {
        let edit = edit.clone();
        tokio::task::spawn_blocking(move || {
            let root = registry::hkey_for(edit.hive);
            // Capture before writing. This ordering is the revert guarantee: if the
            // write fails we have still recorded nothing that needs undoing, and if it
            // succeeds the journal already holds the truth.
            let prior = registry::capture_prior(root, &edit.path, &edit.name)?;
            registry::write_value(root, &edit.path, &edit.name, &edit.value)?;
            Ok(prior)
        })
        .await
        .map_err(|e| sio_core::Error::Other(format!("registry task panicked: {e}")))?
    }

    async fn registry_restore(
        &self,
        hive: Hive,
        path: &str,
        name: &str,
        prior: &PriorValue,
    ) -> Result<()> {
        let (path, name, prior) = (path.to_string(), name.to_string(), prior.clone());
        tokio::task::spawn_blocking(move || {
            registry::restore(registry::hkey_for(hive), &path, &name, &prior)
        })
        .await
        .map_err(|e| sio_core::Error::Other(format!("registry task panicked: {e}")))?
    }

    async fn service_configure(&self, cfg: &ServiceConfig) -> Result<PriorState> {
        let cfg = cfg.clone();
        tokio::task::spawn_blocking(move || {
            let prior = services::query(&cfg.name)?;
            services::set_start_type(&cfg.name, cfg.start_type)?;
            if cfg.stop {
                services::stop(&cfg.name)?;
            }
            Ok(prior)
        })
        .await
        .map_err(|e| sio_core::Error::Other(format!("service task panicked: {e}")))?
    }

    async fn service_restore(&self, prior: &PriorState) -> Result<()> {
        let prior = prior.clone();
        tokio::task::spawn_blocking(move || services::restore(&prior))
            .await
            .map_err(|e| sio_core::Error::Other(format!("service task panicked: {e}")))?
    }

    async fn appx_remove(&self, pkg: &AppxRef) -> Result<()> {
        system::appx_remove(pkg).await
    }

    async fn create_restore_point(&self, description: &str) -> Result<RestorePointOutcome> {
        system::create_restore_point(description).await
    }

    async fn run_package_cmd(&self, cmd: &PackageCmd, progress: ProgressSink) -> Result<i32> {
        process::run_streaming(&cmd.program, &cmd.args, &progress).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sio_core::tweak::RegistryValue;
    use windows::Win32::System::Registry::HKEY_CURRENT_USER;

    const TEST_KEY: &str = r"Software\SioTest\OpsUnitTests";

    fn edit(name: &str, value: u32) -> RegistryEdit {
        RegistryEdit {
            hive: Hive::Hkcu,
            path: TEST_KEY.into(),
            name: name.into(),
            value: RegistryValue::Dword(value),
        }
    }

    #[tokio::test]
    async fn registry_set_returns_the_state_from_before_the_write() {
        let ops = InProcessOps::new();
        let name = "OpsPrior";
        let _ = registry::delete_value(HKEY_CURRENT_USER, TEST_KEY, name);

        // First write: nothing was there.
        let prior = ops.registry_set(&edit(name, 1)).await.unwrap();
        assert!(
            matches!(prior, PriorValue::Absent | PriorValue::KeyAbsent),
            "expected an absent prior state, got {prior:?}"
        );

        // Second write: the previous value must come back, not the new one.
        let prior = ops.registry_set(&edit(name, 2)).await.unwrap();
        assert_eq!(prior, PriorValue::Present(RegistryValue::Dword(1)));

        let _ = registry::delete_value(HKEY_CURRENT_USER, TEST_KEY, name);
    }

    #[tokio::test]
    async fn apply_then_restore_round_trips_through_the_trait() {
        let ops = InProcessOps::new();
        let name = "OpsRoundTrip";
        let _ = registry::delete_value(HKEY_CURRENT_USER, TEST_KEY, name);

        let prior = ops.registry_set(&edit(name, 99)).await.unwrap();
        assert_eq!(
            registry::read_value(HKEY_CURRENT_USER, TEST_KEY, name).unwrap(),
            Some(RegistryValue::Dword(99))
        );

        ops.registry_restore(Hive::Hkcu, TEST_KEY, name, &prior)
            .await
            .unwrap();
        assert_eq!(
            registry::read_value(HKEY_CURRENT_USER, TEST_KEY, name).unwrap(),
            None,
            "restoring an absent prior must delete the value"
        );
    }

    #[tokio::test]
    async fn run_package_cmd_reports_the_exit_code_verbatim() {
        let ops = InProcessOps::new();
        let cmd = PackageCmd {
            provider: sio_core::package::ProviderId::Winget,
            op: sio_core::package::PackageOp::Install,
            program: "cmd".into(),
            args: vec!["/C".into(), "exit 5".into()],
            elevated: false,
        };

        let code = ops
            .run_package_cmd(&cmd, ProgressSink::null())
            .await
            .unwrap();
        assert_eq!(code, 5, "classification is the provider's job, not ours");
    }
}
