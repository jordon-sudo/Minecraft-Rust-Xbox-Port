//! CoreWindow events, collected on the dispatcher thread and drained once a
//! frame.
//!
//! WinRT delivers these through handlers it invokes during
//! `CoreDispatcher::ProcessEvents`. The handlers cannot borrow the `Window`
//! (they outlive the call and WinRT owns them), so they push into a shared
//! queue that [`Window::pump_events`](crate::Window::pump_events) drains
//! immediately afterwards, on the same thread.

use std::sync::{Arc, Mutex};

use windows::{
    Foundation::TypedEventHandler,
    UI::Core::{
        CoreWindow, KeyEventArgs, PointerEventArgs, WindowActivatedEventArgs,
        WindowSizeChangedEventArgs,
    },
};

/// A key, as the platform reports it.
///
/// This is the WinRT virtual key code, kept as a plain number rather than
/// mapped here: the mapping to Horizon's own key enum belongs with the rest
/// of the key mapping, not in the platform layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// What the window reported since the last frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    /// Physical pixels, scale already applied.
    Resized {
        width: u32,
        height: u32,
    },
    /// The app came to the foreground or left it. On Xbox this also fires
    /// when a system overlay (the guide, a sign-in prompt) takes focus, which
    /// is when a single-player game should pause.
    Focused(bool),
    KeyDown(Key),
    KeyUp(Key),
    /// Position is in physical pixels.
    PointerMoved {
        x: f64,
        y: f64,
    },
    PointerPressed {
        button: MouseButton,
        x: f64,
        y: f64,
    },
    PointerReleased {
        button: MouseButton,
        x: f64,
        y: f64,
    },
    /// The user asked to close the window.
    CloseRequested,
}

// `Arc<Mutex<_>>`, not `Rc<RefCell<_>>`: WinRT requires its handlers to be
// agile, so the closures below must be `Send + Sync` even though every one
// of them runs on the dispatcher thread.
type Queue = Arc<Mutex<Vec<Event>>>;

/// Owns the queue and the registered handlers.
pub(crate) struct EventSink {
    queue: Queue,
}

impl EventSink {
    pub(crate) fn new() -> Self {
        Self {
            queue: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn drain(&self) -> Vec<Event> {
        match self.queue.lock() {
            Ok(mut queue) => std::mem::take(&mut *queue),
            Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
        }
    }

    pub(crate) fn attach(&self, window: &CoreWindow) -> windows::core::Result<()> {
        // `scale` is read once here: a CoreWindow's bounds and pointer
        // positions are both in DIPs, so one factor converts both.
        let scale = crate::window::window_scale();

        let q = self.queue.clone();
        window.SizeChanged(
            &TypedEventHandler::<CoreWindow, WindowSizeChangedEventArgs>::new(move |_, args| {
                if let Some(args) = args.as_ref() {
                    let size = args.Size()?;
                    push(
                        &q,
                        Event::Resized {
                            width: ((f64::from(size.Width) * scale).round() as u32).max(1),
                            height: ((f64::from(size.Height) * scale).round() as u32).max(1),
                        },
                    );
                }
                Ok(())
            }),
        )?;

        let q = self.queue.clone();
        window.Activated(
            &TypedEventHandler::<CoreWindow, WindowActivatedEventArgs>::new(move |_, args| {
                if let Some(args) = args.as_ref() {
                    // CoreWindowActivationState::Deactivated == 0
                    let focused = args.WindowActivationState()?.0 != 0;
                    push(&q, Event::Focused(focused));
                }
                Ok(())
            }),
        )?;

        let q = self.queue.clone();
        window.KeyDown(&TypedEventHandler::<CoreWindow, KeyEventArgs>::new(
            move |_, args| {
                if let Some(args) = args.as_ref() {
                    push(&q, Event::KeyDown(Key(args.VirtualKey()?.0 as u32)));
                }
                Ok(())
            },
        ))?;

        let q = self.queue.clone();
        window.KeyUp(&TypedEventHandler::<CoreWindow, KeyEventArgs>::new(
            move |_, args| {
                if let Some(args) = args.as_ref() {
                    push(&q, Event::KeyUp(Key(args.VirtualKey()?.0 as u32)));
                }
                Ok(())
            },
        ))?;

        let q = self.queue.clone();
        window.PointerMoved(&TypedEventHandler::<CoreWindow, PointerEventArgs>::new(
            move |_, args| {
                if let Some((x, y)) = pointer_position(args.as_ref(), scale) {
                    push(&q, Event::PointerMoved { x, y });
                }
                Ok(())
            },
        ))?;

        let q = self.queue.clone();
        window.PointerPressed(&TypedEventHandler::<CoreWindow, PointerEventArgs>::new(
            move |_, args| {
                if let Some(args) = args.as_ref() {
                    if let Some((x, y)) = pointer_position(Some(args), scale) {
                        push(
                            &q,
                            Event::PointerPressed {
                                button: pointer_button(args),
                                x,
                                y,
                            },
                        );
                    }
                }
                Ok(())
            },
        ))?;

        let q = self.queue.clone();
        window.PointerReleased(&TypedEventHandler::<CoreWindow, PointerEventArgs>::new(
            move |_, args| {
                if let Some(args) = args.as_ref() {
                    if let Some((x, y)) = pointer_position(Some(args), scale) {
                        push(
                            &q,
                            Event::PointerReleased {
                                button: pointer_button(args),
                                x,
                                y,
                            },
                        );
                    }
                }
                Ok(())
            },
        ))?;

        Ok(())
    }
}

fn push(queue: &Queue, event: Event) {
    match queue.lock() {
        Ok(mut queue) => queue.push(event),
        Err(poisoned) => poisoned.into_inner().push(event),
    }
}

fn pointer_position(args: Option<&PointerEventArgs>, scale: f64) -> Option<(f64, f64)> {
    let point = args?.CurrentPoint().ok()?.Position().ok()?;
    Some((f64::from(point.X) * scale, f64::from(point.Y) * scale))
}

fn pointer_button(args: &PointerEventArgs) -> MouseButton {
    let properties = args
        .CurrentPoint()
        .and_then(|point| point.Properties())
        .ok();
    let Some(properties) = properties else {
        return MouseButton::Left;
    };
    if properties.IsRightButtonPressed().unwrap_or(false) {
        MouseButton::Right
    } else if properties.IsMiddleButtonPressed().unwrap_or(false) {
        MouseButton::Middle
    } else {
        MouseButton::Left
    }
}
