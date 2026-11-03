//! The Xbox (UWP) platform layer.
//!
//! A UWP process is not a Win32 process wearing a different hat: it has no
//! `HWND`, no Win32 message pump and no free run of the filesystem. Its one
//! top-level window is a WinRT [`CoreWindow`], driven by a `CoreDispatcher`,
//! and its writable storage is the two folders `ApplicationData` hands out.
//! This crate owns those differences so the rest of Horizon does not have to.
//!
//! It deliberately does *not* own the game loop. [`run`] hands control to
//! `CoreApplication`, which calls back into the [`App`] the caller supplies;
//! the caller keeps its own tick-and-render loop and calls
//! [`Window::pump_events`] once a frame, exactly where the desktop build
//! lets winit pump its queue.
//!
//! On every target other than `*-uwp-windows-msvc` this crate is empty, so
//! desktop builds are unaffected.

#![cfg(target_vendor = "uwp")]

mod audio;
mod events;
mod lifecycle;
mod paths;
mod view;
mod window;

pub use audio::{AudioOutput, Fill};
pub use events::{Event, Key, MouseButton};
pub use lifecycle::{Lifecycle, SuspendGuard};
pub use paths::{Paths, local_folder};
pub use view::{App, run};
pub use window::Window;
