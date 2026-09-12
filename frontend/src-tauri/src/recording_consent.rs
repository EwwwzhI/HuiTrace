//! Native enforcement for the pre-recording consent gate.
//!
//! The renderer owns the consent UI, but it cannot start capture by invoking a
//! recording command directly. After the UI confirmation (or a valid persisted
//! acknowledgement), it requests a short-lived authorization and passes the
//! opaque ticket to exactly one recording-start command. Tickets are held only
//! in memory, expire quickly, are bound to the current [`crate::context::AuthContext`],
//! and are consumed before the audio pipeline is entered.

use crate::context;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_store::StoreExt;
use uuid::Uuid;

#[path = "recording_consent_session.rs"]
mod session;

const RECORDING_CONSENT_TICKET_TTL: Duration = Duration::from_secs(60);
const CONSENT_REQUIRED_ERROR: &str =
    "RECORDING_CONSENT_REQUIRED: Confirm participant recording consent before starting capture.";

#[derive(Debug)]
struct RecordingConsentTicket {
    token: String,
    tenant_id: String,
    user_id: String,
    expires_at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TicketValidationError {
    Missing,
    Expired,
    Invalid,
    ContextMismatch,
    StateUnavailable,
}

/// Tauri-managed, process-local authorization state.
///
/// Only one ticket can be outstanding. Issuing a new one invalidates any older
/// ticket, which also makes duplicate UI start attempts fail closed.
#[derive(Debug, Default)]
pub struct RecordingConsentTicketState {
    current: Mutex<Option<RecordingConsentTicket>>,
    // Serialize one-use dialog approvals; never persist this acknowledgment or accept it
    // as an authorization ticket. A new process always starts unconfirmed.
    session: tokio::sync::Mutex<session::ConsentSession>,
}

impl RecordingConsentTicketState {
    fn issue_at(&self, tenant_id: &str, user_id: &str, now: Instant) -> Result<String, String> {
        let token = Uuid::new_v4().to_string();
        let ticket = RecordingConsentTicket {
            token: token.clone(),
            tenant_id: tenant_id.to_owned(),
            user_id: user_id.to_owned(),
            expires_at: now + RECORDING_CONSENT_TICKET_TTL,
        };

        let mut current = self.current.lock().map_err(|_| {
            "Recording consent authorization is temporarily unavailable.".to_string()
        })?;
        *current = Some(ticket);
        Ok(token)
    }

    fn consume_at(
        &self,
        token: &str,
        tenant_id: &str,
        user_id: &str,
        now: Instant,
    ) -> Result<(), TicketValidationError> {
        let mut current = self
            .current
            .lock()
            .map_err(|_| TicketValidationError::StateUnavailable)?;

        let Some(ticket) = current.take() else {
            return Err(TicketValidationError::Missing);
        };

        if now >= ticket.expires_at {
            return Err(TicketValidationError::Expired);
        }
        if ticket.token != token {
            return Err(TicketValidationError::Invalid);
        }
        if ticket.tenant_id != tenant_id || ticket.user_id != user_id {
            return Err(TicketValidationError::ContextMismatch);
        }

        Ok(())
    }
}

/// Record an explicit confirmation from the app's themed consent dialog.
/// This approval is consumed by the next authorization request only.
#[tauri::command]
pub async fn confirm_recording_consent<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    let ctx = context::current();
    let state = app.state::<RecordingConsentTicketState>();
    state
        .session
        .lock()
        .await
        .confirm(ctx.tenant_id.as_str(), ctx.user_id.as_str());
    Ok(())
}

/// Issue a ticket only for a remembered preference or a fresh dialog approval.
#[tauri::command]
pub async fn authorize_recording_start<R: Runtime>(app: AppHandle<R>) -> Result<String, String> {
    let ctx = context::current();
    let state = app.state::<RecordingConsentTicketState>();
    let confirmed = state
        .session
        .lock()
        .await
        .take_confirmation(ctx.tenant_id.as_str(), ctx.user_id.as_str());
    let remembered = app
        .store("recording-consent.json")
        .ok()
        .is_some_and(|store| {
            store
                .get("recordingConsentAcknowledged")
                .and_then(|v| v.as_bool())
                == Some(true)
                && store
                    .get("recordingConsentAlwaysAsk")
                    .and_then(|v| v.as_bool())
                    != Some(true)
        });
    if !confirmed && !remembered {
        return Err(CONSENT_REQUIRED_ERROR.to_string());
    }
    state.issue_at(ctx.tenant_id.as_str(), ctx.user_id.as_str(), Instant::now())
}
/// Consume an authorization before entering any recording-start implementation.
pub fn consume_recording_start_authorization<R: Runtime>(
    app: &AppHandle<R>,
    token: &str,
) -> Result<(), String> {
    let ctx = context::current();
    let state = app.state::<RecordingConsentTicketState>();

    state
        .consume_at(
            token,
            ctx.tenant_id.as_str(),
            ctx.user_id.as_str(),
            Instant::now(),
        )
        .map_err(|error| {
            log::warn!(
                "Recording start rejected by native consent gate: {:?}",
                error
            );
            CONSENT_REQUIRED_ERROR.to_string()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TENANT: &str = "workspace-a";
    const USER: &str = "user-a";

    #[test]
    fn absent_ticket_fails_closed() {
        let state = RecordingConsentTicketState::default();

        assert_eq!(
            state.consume_at("not-issued", TENANT, USER, Instant::now()),
            Err(TicketValidationError::Missing)
        );
    }

    #[test]
    fn expired_ticket_fails_closed_and_is_removed() {
        let state = RecordingConsentTicketState::default();
        let issued_at = Instant::now();
        let token = state.issue_at(TENANT, USER, issued_at).unwrap();

        assert_eq!(
            state.consume_at(
                &token,
                TENANT,
                USER,
                issued_at + RECORDING_CONSENT_TICKET_TTL
            ),
            Err(TicketValidationError::Expired)
        );
        assert_eq!(
            state.consume_at(&token, TENANT, USER, issued_at),
            Err(TicketValidationError::Missing)
        );
    }

    #[test]
    fn ticket_is_single_use() {
        let state = RecordingConsentTicketState::default();
        let issued_at = Instant::now();
        let token = state.issue_at(TENANT, USER, issued_at).unwrap();

        assert_eq!(state.consume_at(&token, TENANT, USER, issued_at), Ok(()));
        assert_eq!(
            state.consume_at(&token, TENANT, USER, issued_at),
            Err(TicketValidationError::Missing)
        );
    }

    #[test]
    fn ticket_is_bound_to_auth_context() {
        let state = RecordingConsentTicketState::default();
        let issued_at = Instant::now();
        let token = state.issue_at(TENANT, USER, issued_at).unwrap();

        assert_eq!(
            state.consume_at(&token, "workspace-b", USER, issued_at),
            Err(TicketValidationError::ContextMismatch)
        );
    }

    #[test]
    fn presenting_the_wrong_token_consumes_the_outstanding_ticket() {
        let state = RecordingConsentTicketState::default();
        let issued_at = Instant::now();
        let token = state.issue_at(TENANT, USER, issued_at).unwrap();

        assert_eq!(
            state.consume_at("wrong-token", TENANT, USER, issued_at),
            Err(TicketValidationError::Invalid)
        );
        assert_eq!(
            state.consume_at(&token, TENANT, USER, issued_at),
            Err(TicketValidationError::Missing)
        );
    }
}
