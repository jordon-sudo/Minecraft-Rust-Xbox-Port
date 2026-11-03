//! Suspend and resume.
//!
//! Xbox suspends an app that loses the foreground and gives it a short,
//! enforced budget to get out of the way; an app that overruns is killed.
//! Horizon must therefore flush the world to disk *inside* the Suspending
//! handler, not on the next tick, because there may not be a next tick.
//!
//! WinRT lets a handler extend that budget by taking a deferral.
//! [`SuspendGuard`] is that deferral: hold it while saving, drop it when
//! done. Dropping it is what tells the system the app is ready; leaking one
//! stalls the suspend until the system loses patience and terminates.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use windows::{
    ApplicationModel::Core::CoreApplication,
    ApplicationModel::{SuspendingDeferral, SuspendingEventArgs},
    Foundation::EventHandler,
    core::IInspectable,
};

/// Held for as long as the app needs to finish saving. Drop it to let the
/// suspend proceed.
pub struct SuspendGuard(Option<SuspendingDeferral>);

impl Drop for SuspendGuard {
    fn drop(&mut self) {
        if let Some(deferral) = self.0.take()
            && let Err(e) = deferral.Complete()
        {
            log::error!("suspend deferral Complete failed: {e}");
        }
    }
}

/// Watches the app's suspend and resume notifications.
///
/// The flags are atomics rather than a callback because the handlers run on
/// a system thread, while the save has to happen on the game's own thread:
/// the game loop polls [`take_suspending`](Self::take_suspending) once a
/// frame and does the work where its state lives.
#[derive(Clone)]
pub struct Lifecycle {
    suspending: Arc<AtomicBool>,
    resumed: Arc<AtomicBool>,
    deferral: Arc<std::sync::Mutex<Option<SuspendingDeferral>>>,
}

impl Lifecycle {
    pub fn new() -> windows::core::Result<Self> {
        let this = Self {
            suspending: Arc::new(AtomicBool::new(false)),
            resumed: Arc::new(AtomicBool::new(false)),
            deferral: Arc::new(std::sync::Mutex::new(None)),
        };

        let suspending = this.suspending.clone();
        let slot = this.deferral.clone();
        CoreApplication::Suspending(&EventHandler::<SuspendingEventArgs>::new(move |_, args| {
            // Take the deferral first: until it is held, the budget is
            // already running.
            if let Some(args) = args.as_ref()
                && let Ok(deferral) = args.SuspendingOperation().and_then(|op| op.GetDeferral())
            {
                if let Ok(mut slot) = slot.lock() {
                    *slot = Some(deferral);
                }
            }
            suspending.store(true, Ordering::SeqCst);
            Ok(())
        }))?;

        let resumed = this.resumed.clone();
        CoreApplication::Resuming(&EventHandler::<IInspectable>::new(move |_, _| {
            resumed.store(true, Ordering::SeqCst);
            Ok(())
        }))?;

        Ok(this)
    }

    /// Returns a guard once, the first time the app is suspending. Save the
    /// world while it is alive, then drop it.
    pub fn take_suspending(&self) -> Option<SuspendGuard> {
        if !self.suspending.swap(false, Ordering::SeqCst) {
            return None;
        }
        let deferral = self.deferral.lock().ok().and_then(|mut slot| slot.take());
        Some(SuspendGuard(deferral))
    }

    /// True once per resume. The GPU device may have been lost while the app
    /// was away, so the caller should expect to rebuild the surface.
    pub fn take_resumed(&self) -> bool {
        self.resumed.swap(false, Ordering::SeqCst)
    }
}
