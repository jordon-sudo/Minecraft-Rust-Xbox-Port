//! The `CoreApplication` entry point.
//!
//! UWP inverts startup: the process does not open a window, it hands
//! `CoreApplication` a view and is called back. `IFrameworkView::Run` is the
//! one place the app owns the thread, so that is where Horizon's loop goes.
//!
//! The game itself is not written here. The caller passes an [`App`], and
//! this module calls it once with a ready [`Window`] and [`Lifecycle`]; what
//! the caller does with the thread after that is its own business.

use std::cell::RefCell;

use windows::{
    ApplicationModel::Core::{
        CoreApplication, CoreApplicationView, IFrameworkView, IFrameworkView_Impl,
        IFrameworkViewSource, IFrameworkViewSource_Impl,
    },
    UI::Core::CoreWindow,
    core::{HSTRING, Ref, Result, implement},
};

use crate::{lifecycle::Lifecycle, window::Window};

/// What the caller runs once the platform is ready.
///
/// Returning hands control back to the system and ends the app.
pub trait App {
    fn run(&mut self, window: Window, lifecycle: Lifecycle) -> Result<()>;
}

impl<F> App for F
where
    F: FnMut(Window, Lifecycle) -> Result<()>,
{
    fn run(&mut self, window: Window, lifecycle: Lifecycle) -> Result<()> {
        self(window, lifecycle)
    }
}

/// Hands control to `CoreApplication` and does not return until the app
/// exits. Call it from `main` on the process's first thread.
pub fn run(app: impl App + 'static) -> Result<()> {
    let source: IFrameworkViewSource = ViewSource {
        app: RefCell::new(Some(Box::new(app))),
    }
    .into();
    CoreApplication::Run(&source)
}

#[implement(IFrameworkViewSource)]
struct ViewSource {
    app: RefCell<Option<Box<dyn App>>>,
}

impl IFrameworkViewSource_Impl for ViewSource_Impl {
    fn CreateView(&self) -> Result<IFrameworkView> {
        // The system asks for exactly one view; a second request means
        // something is wrong, and handing out a view with no app behind it
        // would deadlock at Run.
        let app = self.app.borrow_mut().take().ok_or_else(|| {
            windows::core::Error::new(
                windows::Win32::Foundation::E_UNEXPECTED,
                "CreateView called more than once",
            )
        })?;
        Ok(View {
            app: RefCell::new(Some(app)),
            window: RefCell::new(None),
        }
        .into())
    }
}

#[implement(IFrameworkView)]
struct View {
    app: RefCell<Option<Box<dyn App>>>,
    window: RefCell<Option<CoreWindow>>,
}

impl IFrameworkView_Impl for View_Impl {
    fn Initialize(&self, _view: Ref<CoreApplicationView>) -> Result<()> {
        Ok(())
    }

    fn SetWindow(&self, window: Ref<CoreWindow>) -> Result<()> {
        // Keep it: Run needs it, and this is the only time it is offered.
        *self.window.borrow_mut() = window.as_ref().cloned();
        Ok(())
    }

    fn Load(&self, _entry_point: &HSTRING) -> Result<()> {
        Ok(())
    }

    fn Run(&self) -> Result<()> {
        let core_window = self.window.borrow().clone().ok_or_else(|| {
            windows::core::Error::new(
                windows::Win32::Foundation::E_UNEXPECTED,
                "Run before SetWindow",
            )
        })?;
        // Nothing is drawn until the window is activated, and on Xbox an
        // app that never activates never gets input either.
        core_window.Activate()?;

        let window = Window::new(core_window)?;
        let lifecycle = Lifecycle::new()?;

        let mut app = self.app.borrow_mut().take().ok_or_else(|| {
            windows::core::Error::new(windows::Win32::Foundation::E_UNEXPECTED, "Run called twice")
        })?;
        app.run(window, lifecycle)
    }

    fn Uninitialize(&self) -> Result<()> {
        Ok(())
    }
}
