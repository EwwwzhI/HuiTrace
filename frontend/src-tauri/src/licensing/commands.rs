//! HuiTrace has no commercial trial. Legacy IPC is retained without reading
//! trial storage, modifying saved keys, or contacting the upstream service.
use super::{LicensingStateKind, LicensingStatus};
use tauri::{AppHandle, Runtime};

fn unrestricted_status() -> LicensingStatus {
    LicensingStatus {
        // Legacy wire format has no "not_required" state.
        state: LicensingStateKind::Licensed,
        days_left: None,
        plan: None,
        expires_at: None,
        display_key: None,
        reason: None,
        configured: false,
    }
}

#[tauri::command]
pub async fn get_licensing_status<R: Runtime>(_app: AppHandle<R>) -> LicensingStatus {
    unrestricted_status()
}

#[tauri::command]
pub async fn activate_license<R: Runtime>(
    _app: AppHandle<R>,
    key: String,
) -> Result<LicensingStatus, String> {
    let _ = key;
    Err("NOT_CONFIGURED: HuiTrace does not require a license key.".to_string())
}

#[tauri::command]
pub async fn deactivate_license<R: Runtime>(_app: AppHandle<R>) -> Result<LicensingStatus, String> {
    Ok(unrestricted_status())
}

/// Capture does not depend on any legacy trial or key state.
pub async fn ensure_capture_allowed<R: Runtime>(_app: &AppHandle<R>) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_has_no_trial_expiry_or_commercial_configuration() {
        let status = unrestricted_status();
        assert_eq!(status.state, LicensingStateKind::Licensed);
        assert!(!status.configured);
        assert!(status.days_left.is_none());
        assert!(status.expires_at.is_none());
        assert!(status.display_key.is_none());
    }
}
