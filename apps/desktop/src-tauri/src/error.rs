//! Error projection for the IPC boundary.
//!
//! [`sio_core::Error`] messages are English and written for logs. The UI ships in three
//! languages, so commands return a stable machine-readable `code` that the frontend
//! translates, plus the original text for the log pane and bug reports. Sending only a
//! formatted string would make every error untranslatable.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    /// Stable identifier the frontend maps to a translated message. Never rename these
    /// without updating all locale files.
    pub code: &'static str,
    /// Untranslated detail. Shown in the log pane, not as the primary message.
    pub detail: String,
}

impl From<sio_core::Error> for CommandError {
    fn from(err: sio_core::Error) -> Self {
        use sio_core::Error as E;
        let code = match &err {
            E::Io(_) => "io",
            E::Json(_) => "json",
            E::Catalog { .. } => "catalogInvalid",
            E::ProviderUnavailable { .. } => "providerUnavailable",
            E::PackageCommand { .. } => "packageCommandFailed",
            E::ElevationDeclined => "elevationDeclined",
            E::Broker { .. } => "brokerFailed",
            E::Registry { .. } => "registryFailed",
            E::Windows { .. } => "windowsApiFailed",
            E::UnknownTweak { .. } => "unknownTweak",
            E::Other(_) => "unknown",
        };
        Self {
            code,
            detail: err.to_string(),
        }
    }
}

pub type CommandResult<T> = std::result::Result<T, CommandError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elevation_declined_gets_its_own_code() {
        // The UI shows this as an ordinary "cancelled" notice, not an error dialog,
        // so it must be distinguishable from a genuine broker failure.
        let err = CommandError::from(sio_core::Error::ElevationDeclined);
        assert_eq!(err.code, "elevationDeclined");

        let broker = CommandError::from(sio_core::Error::Broker {
            reason: "crashed".into(),
        });
        assert_ne!(err.code, broker.code);
    }

    #[test]
    fn serialises_with_camel_case_keys() {
        let err = CommandError::from(sio_core::Error::Other("boom".into()));
        let json = serde_json::to_string(&err).unwrap();
        assert!(json.contains("\"code\""), "got {json}");
        assert!(json.contains("\"detail\""), "got {json}");
    }
}
