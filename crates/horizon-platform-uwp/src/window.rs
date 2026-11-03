//! The `CoreWindow`: its size in physical pixels, its scale factor, and the
//! raw handle wgpu needs to build a DX12 swapchain from it.

use std::ptr::NonNull;

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WinRtWindowHandle, WindowHandle, WindowsDisplayHandle,
};
use windows::{
    Graphics::Display::DisplayInformation,
    UI::Core::{CoreProcessEventsOption, CoreWindow},
    core::Interface,
};

use crate::events::{Event, EventSink};

/// The process's one top-level window.
///
/// `CoreWindow` is apartment-bound: it belongs to the thread
/// `IFrameworkView::SetWindow` handed it to, and every method here must run on
/// that thread. The type is therefore deliberately neither `Send` nor `Sync`.
pub struct Window {
    core_window: CoreWindow,
    sink: EventSink,
    /// Cached so a frame that asks for the size does not cross the WinRT
    /// boundary four times; refreshed when the window reports a resize.
    size: (u32, u32),
    scale: f64,
}

impl Window {
    pub(crate) fn new(core_window: CoreWindow) -> windows::core::Result<Self> {
        let scale = window_scale();
        let size = Self::read_size(&core_window, scale);
        let sink = EventSink::new();
        sink.attach(&core_window)?;
        Ok(Self {
            core_window,
            sink,
            size,
            scale,
        })
    }

    /// The window's size in **physical pixels**.
    ///
    /// `CoreWindow::Bounds` is in device-independent pixels, so on a 4K Xbox
    /// output it reports 1920x1080 at a scale of 2.0. wgpu wants pixels, so
    /// the scale factor is applied here and the rest of Horizon sees the same
    /// kind of number the desktop build gets from winit.
    pub fn inner_size(&self) -> (u32, u32) {
        self.size
    }

    /// DIPs to physical pixels. 1.0 at 1080p, 2.0 at 4K.
    pub fn scale_factor(&self) -> f64 {
        self.scale
    }

    /// Runs every event the dispatcher has queued, then returns. Call once a
    /// frame; it never blocks, so the caller keeps pacing its own loop.
    pub fn pump_events(&mut self, mut handler: impl FnMut(Event)) -> windows::core::Result<()> {
        self.core_window
            .Dispatcher()?
            .ProcessEvents(CoreProcessEventsOption::ProcessAllIfPresent)?;
        for event in self.sink.drain() {
            if let Event::Resized { width, height } = event {
                self.size = (width, height);
            }
            handler(event);
        }
        Ok(())
    }

    /// Re-reads the size and scale from the system. The resize events already
    /// keep [`inner_size`](Self::inner_size) current; this is for the output
    /// mode changing under the app (1080p to 4K), which moves the scale
    /// factor without necessarily moving the bounds.
    pub fn refresh_scale(&mut self) {
        self.scale = window_scale();
        self.size = Self::read_size(&self.core_window, self.scale);
    }

    fn read_size(core_window: &CoreWindow, scale: f64) -> (u32, u32) {
        let bounds = match core_window.Bounds() {
            Ok(bounds) => bounds,
            Err(e) => {
                log::warn!("CoreWindow::Bounds failed: {e}");
                return (1, 1);
            }
        };
        // A zero-sized surface is not configurable, so never report one.
        let width = ((f64::from(bounds.Width) * scale).round() as u32).max(1);
        let height = ((f64::from(bounds.Height) * scale).round() as u32).max(1);
        (width, height)
    }

    /// The CoreWindow as a raw handle, for wgpu's unsafe surface target.
    ///
    /// wgpu's safe `create_surface` wants a target that is `Send + Sync`; a
    /// CoreWindow is apartment-bound and deliberately neither, so callers go
    /// through `create_surface_unsafe` with these two. The surface must not
    /// outlive this `Window`.
    pub fn raw_window_handle(&self) -> Result<RawWindowHandle, HandleError> {
        let ptr = NonNull::new(self.core_window.as_raw()).ok_or(HandleError::Unavailable)?;
        Ok(RawWindowHandle::WinRt(WinRtWindowHandle::new(ptr)))
    }

    /// The display handle that goes with it. wgpu's DX12 backend asserts this
    /// is the Windows one.
    pub fn raw_display_handle(&self) -> RawDisplayHandle {
        RawDisplayHandle::Windows(WindowsDisplayHandle::new())
    }
}

impl HasWindowHandle for Window {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        // Borrowed, not owned: `self` keeps the CoreWindow alive, and the
        // handle cannot outlive the borrow.
        let ptr = self.core_window.as_raw();
        let ptr = NonNull::new(ptr).ok_or(HandleError::Unavailable)?;
        let handle = WinRtWindowHandle::new(ptr);
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::WinRt(handle)) })
    }
}

impl HasDisplayHandle for Window {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        // wgpu's DX12 backend asserts the display handle is the Windows one.
        Ok(unsafe {
            DisplayHandle::borrow_raw(RawDisplayHandle::Windows(WindowsDisplayHandle::new()))
        })
    }
}

/// DIPs to physical pixels for the current view.
///
/// Free rather than a method because the event handlers need it too, and
/// they cannot borrow the `Window` they feed.
pub(crate) fn window_scale() -> f64 {
    DisplayInformation::GetForCurrentView()
        .and_then(|info| info.LogicalDpi())
        .map(|dpi| f64::from(dpi) / 96.0)
        .unwrap_or(1.0)
}
