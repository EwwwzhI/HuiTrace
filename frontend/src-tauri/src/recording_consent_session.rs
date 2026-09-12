//! One-use approval from the app consent dialog, bound to the auth context.
use std::time::{Duration, Instant};
#[derive(Debug, Default)]
pub(crate) struct ConsentSession {
    confirmed: Option<(String, String, Instant)>,
}
impl ConsentSession {
    pub(crate) fn take_confirmation(&mut self, tenant: &str, user: &str) -> bool {
        self.confirmed
            .take()
            .is_some_and(|(t, u, expiry)| t == tenant && u == user && Instant::now() < expiry)
    }
    pub(crate) fn confirm(&mut self, tenant: &str, user: &str) {
        self.confirmed = Some((
            tenant.to_owned(),
            user.to_owned(),
            Instant::now() + Duration::from_secs(60),
        ));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn approval_is_required_and_single_use() {
        let mut state = ConsentSession::default();
        assert!(!state.take_confirmation("t", "u"));
        state.confirm("t", "u");
        assert!(state.take_confirmation("t", "u"));
        assert!(!state.take_confirmation("t", "u"));
    }
    #[test]
    fn wrong_context_and_expired_approval_fail_closed() {
        let mut state = ConsentSession::default();
        state.confirm("t", "u");
        assert!(!state.take_confirmation("other", "u"));
        assert!(!state.take_confirmation("t", "u"));
        state.confirmed = Some(("t".into(), "u".into(), Instant::now()));
        assert!(!state.take_confirmation("t", "u"));
    }
}
