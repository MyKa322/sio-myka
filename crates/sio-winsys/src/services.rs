//! Windows service configuration.
//!
//! Uses the Service Control Manager directly rather than shelling out to `sc.exe` or
//! `Get-Service`. Two reasons: `sc.exe` prints localized text that would have to be
//! parsed, and the SCM reports start types as numbers, which are identical on every
//! Windows language.
//!
//! Reading configuration needs no elevation; changing it does.

use sio_core::error::{Error, Result};
use sio_core::tweak::{PriorState, ServiceStartType};
use windows::core::HSTRING;
use windows::Win32::System::Services::{
    ChangeServiceConfigW, CloseServiceHandle, ControlService, OpenSCManagerW, OpenServiceW,
    QueryServiceConfigW, QueryServiceStatus, StartServiceW, QUERY_SERVICE_CONFIGW, SC_HANDLE,
    SC_MANAGER_CONNECT, SERVICE_AUTO_START, SERVICE_BOOT_START, SERVICE_CHANGE_CONFIG,
    SERVICE_CONTROL_STOP, SERVICE_DEMAND_START, SERVICE_DISABLED, SERVICE_ERROR,
    SERVICE_QUERY_CONFIG, SERVICE_QUERY_STATUS, SERVICE_START, SERVICE_START_TYPE, SERVICE_STATUS,
    SERVICE_STOP, SERVICE_STOPPED, SERVICE_SYSTEM_START,
};

/// Passed where a `ChangeServiceConfigW` field should be left alone.
const SERVICE_NO_CHANGE: u32 = 0xFFFF_FFFF;

/// How long to wait for a service to actually stop before giving up.
const STOP_TIMEOUT_MS: u64 = 10_000;

struct OwnedScHandle(SC_HANDLE);

impl Drop for OwnedScHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseServiceHandle(self.0);
        }
    }
}

fn to_start_type(raw: SERVICE_START_TYPE) -> ServiceStartType {
    match raw {
        SERVICE_BOOT_START => ServiceStartType::Boot,
        SERVICE_SYSTEM_START => ServiceStartType::System,
        SERVICE_AUTO_START => ServiceStartType::Automatic,
        SERVICE_DISABLED => ServiceStartType::Disabled,
        // SERVICE_DEMAND_START and anything unrecognised.
        _ => ServiceStartType::Manual,
    }
}

fn from_start_type(value: ServiceStartType) -> SERVICE_START_TYPE {
    match value {
        ServiceStartType::Boot => SERVICE_BOOT_START,
        ServiceStartType::System => SERVICE_SYSTEM_START,
        ServiceStartType::Automatic => SERVICE_AUTO_START,
        ServiceStartType::Manual => SERVICE_DEMAND_START,
        ServiceStartType::Disabled => SERVICE_DISABLED,
    }
}

fn service_error(name: &str, api: &str, e: windows::core::Error) -> Error {
    Error::Windows {
        api: api.into(),
        reason: format!("service `{name}`: {e}"),
    }
}

fn open(name: &str, access: u32) -> Result<(OwnedScHandle, OwnedScHandle)> {
    unsafe {
        let scm = OpenSCManagerW(None, None, SC_MANAGER_CONNECT)
            .map_err(|e| service_error(name, "OpenSCManagerW", e))?;
        let scm = OwnedScHandle(scm);

        let service = OpenServiceW(scm.0, &HSTRING::from(name), access)
            .map_err(|e| service_error(name, "OpenServiceW", e))?;
        Ok((scm, OwnedScHandle(service)))
    }
}

/// Read a service's current start type and whether it is running.
pub fn query(name: &str) -> Result<PriorState> {
    let (_scm, service) = open(name, SERVICE_QUERY_CONFIG | SERVICE_QUERY_STATUS)?;

    // Two-call pattern: the config struct carries trailing variable-length strings.
    let mut needed = 0u32;
    unsafe {
        // The first call is expected to fail with ERROR_INSUFFICIENT_BUFFER; we only
        // want the size it reports.
        let _ = QueryServiceConfigW(service.0, None, 0, &mut needed);
    }

    let mut buffer =
        vec![0u8; needed.max(std::mem::size_of::<QUERY_SERVICE_CONFIGW>() as u32) as usize];
    let config_ptr = buffer.as_mut_ptr().cast::<QUERY_SERVICE_CONFIGW>();
    unsafe {
        QueryServiceConfigW(
            service.0,
            Some(config_ptr),
            buffer.len() as u32,
            &mut needed,
        )
        .map_err(|e| service_error(name, "QueryServiceConfigW", e))?;
    }
    let start_type = to_start_type(unsafe { (*config_ptr).dwStartType });

    let mut status = SERVICE_STATUS::default();
    unsafe {
        QueryServiceStatus(service.0, &mut status)
            .map_err(|e| service_error(name, "QueryServiceStatus", e))?;
    }

    Ok(PriorState {
        name: name.to_string(),
        start_type,
        was_running: status.dwCurrentState != SERVICE_STOPPED,
    })
}

/// Change a service's start type. Requires elevation.
pub fn set_start_type(name: &str, start_type: ServiceStartType) -> Result<()> {
    let (_scm, service) = open(name, SERVICE_CHANGE_CONFIG)?;
    unsafe {
        ChangeServiceConfigW(
            service.0,
            windows::Win32::System::Services::ENUM_SERVICE_TYPE(SERVICE_NO_CHANGE),
            from_start_type(start_type),
            SERVICE_ERROR(SERVICE_NO_CHANGE),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .map_err(|e| service_error(name, "ChangeServiceConfigW", e))?;
    }
    Ok(())
}

/// Stop a service and wait for it to reach the stopped state.
///
/// Already-stopped is success. Returning as soon as the stop is *requested* would let
/// a following operation race a service that is still shutting down.
pub fn stop(name: &str) -> Result<()> {
    let (_scm, service) = open(name, SERVICE_STOP | SERVICE_QUERY_STATUS)?;

    let mut status = SERVICE_STATUS::default();
    unsafe {
        QueryServiceStatus(service.0, &mut status)
            .map_err(|e| service_error(name, "QueryServiceStatus", e))?;
        if status.dwCurrentState == SERVICE_STOPPED {
            return Ok(());
        }
        ControlService(service.0, SERVICE_CONTROL_STOP, &mut status)
            .map_err(|e| service_error(name, "ControlService", e))?;
    }

    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(STOP_TIMEOUT_MS);
    while std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(200));
        unsafe {
            if QueryServiceStatus(service.0, &mut status).is_err() {
                break;
            }
        }
        if status.dwCurrentState == SERVICE_STOPPED {
            return Ok(());
        }
    }

    Err(Error::Windows {
        api: "ControlService".into(),
        reason: format!("service `{name}` did not stop within {STOP_TIMEOUT_MS}ms"),
    })
}

/// Start a service. Already-running is success.
pub fn start(name: &str) -> Result<()> {
    let (_scm, service) = open(name, SERVICE_START | SERVICE_QUERY_STATUS)?;

    let mut status = SERVICE_STATUS::default();
    unsafe {
        QueryServiceStatus(service.0, &mut status)
            .map_err(|e| service_error(name, "QueryServiceStatus", e))?;
        if status.dwCurrentState != SERVICE_STOPPED {
            return Ok(());
        }
        StartServiceW(service.0, None).map_err(|e| service_error(name, "StartServiceW", e))?;
    }
    Ok(())
}

/// Put a service back to a captured state.
///
/// Start type is restored *before* the running state, because a disabled service cannot
/// be started — doing it the other way round fails on exactly the tweaks people most
/// want to undo.
pub fn restore(prior: &PriorState) -> Result<()> {
    set_start_type(&prior.name, prior.start_type)?;
    if prior.was_running {
        start(&prior.name)?;
    } else {
        stop(&prior.name)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Windows Event Log service. Present and running on every Windows install,
    /// and readable without elevation.
    const WELL_KNOWN: &str = "EventLog";

    #[test]
    fn queries_a_well_known_service() {
        let state = query(WELL_KNOWN).expect("EventLog exists on every Windows install");
        assert_eq!(state.name, WELL_KNOWN);
        assert!(state.was_running, "the event log is always running");
        assert_eq!(state.start_type, ServiceStartType::Automatic);
    }

    #[test]
    fn querying_a_missing_service_is_an_error_not_a_default() {
        // Silently returning a default would make the journal record a state that was
        // never true, and revert would then "restore" a service that never existed.
        assert!(query("SioNoSuchServiceAnywhere").is_err());
    }

    #[test]
    fn start_type_mapping_round_trips() {
        for value in [
            ServiceStartType::Boot,
            ServiceStartType::System,
            ServiceStartType::Automatic,
            ServiceStartType::Manual,
            ServiceStartType::Disabled,
        ] {
            assert_eq!(to_start_type(from_start_type(value)), value);
        }
    }

    #[test]
    fn unknown_raw_start_types_fall_back_to_manual() {
        assert_eq!(
            to_start_type(SERVICE_START_TYPE(99)),
            ServiceStartType::Manual
        );
    }

    /// Mutating a real service needs administrator rights and changes the machine.
    /// Run deliberately, on a throwaway VM, never in CI.
    #[test]
    #[ignore = "requires elevation and modifies a real service"]
    fn change_and_restore_a_service_start_type() {
        let original = query("Spooler").unwrap();
        set_start_type("Spooler", ServiceStartType::Disabled).unwrap();
        assert_eq!(
            query("Spooler").unwrap().start_type,
            ServiceStartType::Disabled
        );

        restore(&original).unwrap();
        assert_eq!(query("Spooler").unwrap().start_type, original.start_type);
    }
}
