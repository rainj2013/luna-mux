use std::sync::atomic::{AtomicU8, Ordering};

const IDLE: u8 = 0;
const CONFIRMING: u8 = 1;
const CLEANING: u8 = 2;
const READY: u8 = 3;

/// One owner for confirmation and cleanup, shared by window-close and app-quit.
#[derive(Default)]
pub struct ExitLifecycle(AtomicU8);

impl ExitLifecycle {
    pub fn request(&self) -> bool {
        self.transition(IDLE, CONFIRMING)
    }

    pub fn cancel(&self) {
        self.transition(CONFIRMING, IDLE);
    }

    pub fn begin_cleanup(&self) -> bool {
        self.transition(CONFIRMING, CLEANING)
    }

    pub fn complete_cleanup(&self) {
        self.transition(CLEANING, READY);
    }

    pub fn can_exit(&self) -> bool {
        self.0.load(Ordering::Acquire) == READY
    }

    fn transition(&self, from: u8, to: u8) -> bool {
        self.0
            .compare_exchange(from, to, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_does_not_start_cleanup_and_allows_retry() {
        let lifecycle = ExitLifecycle::default();
        assert!(lifecycle.request());
        assert!(!lifecycle.can_exit());
        assert!(!lifecycle.request());
        lifecycle.cancel();
        assert!(!lifecycle.can_exit());
        // A cancelled dialog must not leave cleanup eligible to run.
        assert!(!lifecycle.begin_cleanup());
        assert!(lifecycle.request());
        assert!(lifecycle.begin_cleanup());
    }

    #[test]
    fn repeated_quit_cannot_bypass_confirmation_or_in_progress_cleanup() {
        let lifecycle = ExitLifecycle::default();
        assert!(lifecycle.request());
        assert!(!lifecycle.request());
        assert!(!lifecycle.can_exit());
        assert!(lifecycle.begin_cleanup());
        assert!(!lifecycle.begin_cleanup());
        assert!(!lifecycle.request());
        assert!(!lifecycle.can_exit());
        lifecycle.cancel();
        assert!(!lifecycle.request());
        lifecycle.complete_cleanup();
        assert!(lifecycle.can_exit());
        assert!(!lifecycle.request());
    }

    #[test]
    fn idle_runtime_path_still_requires_cleanup_before_exit() {
        let lifecycle = ExitLifecycle::default();
        lifecycle.complete_cleanup();
        assert!(!lifecycle.can_exit());
        assert!(lifecycle.request());
        assert!(lifecycle.begin_cleanup());
        lifecycle.complete_cleanup();
        assert!(lifecycle.can_exit());
    }
}
