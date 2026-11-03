//! A minimal Xbox app: open a CoreWindow, make a Direct3D 12 surface from it,
//! and clear the screen to a slowly changing colour.
//!
//! This exists to answer one question before the game is wired to the platform
//! layer: **does a Rust UWP app run on an Xbox at all?** An open winit issue
//! reports a trivial Rust UWP app that ran on Windows and would not start on a
//! Series S, and if that is true here then every other piece of the port is
//! wasted effort. This uses exactly the pieces the game will use, so what it
//! proves transfers.
//!
//! There is no console output on a retail console, so it writes a log to its
//! own LocalState folder, which Device Portal can download.
//!
//! What a pass looks like: the screen cycles blue to purple, the controller's
//! Menu button closes it, and `smoke.log` ends with "clean exit".

#[cfg(not(target_vendor = "uwp"))]
fn main() {
    eprintln!(
        "horizon-xbox-smoke only does anything on Xbox. Build it with:\n  \
         cargo +nightly build --release -Z build-std=std,panic_abort \\\n    \
         --target x86_64-uwp-windows-msvc -p horizon-xbox-smoke"
    );
}

#[cfg(target_vendor = "uwp")]
fn main() {
    if let Err(e) = horizon_platform_uwp::run(run_app) {
        uwp::log(&format!("CoreApplication::Run failed: {e}"));
    }
}

#[cfg(target_vendor = "uwp")]
use uwp::run_app;

#[cfg(target_vendor = "uwp")]
mod uwp {
    use std::{
        fmt::Write as _,
        io::Write as _,
        time::{Duration, Instant},
    };

    use horizon_platform_uwp::{Event, Lifecycle, Window};

    /// Appends a line to `LocalState\smoke.log`.
    ///
    /// Every step writes one, because when this fails on a console there is
    /// nothing else to go on: the app either appears or it does not, and the
    /// last line written says how far it got.
    pub fn log(line: &str) {
        let Some(local) = horizon_platform_uwp::local_folder() else {
            return;
        };
        let path = local.join("smoke.log");
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = writeln!(file, "{line}");
        }
    }

    pub fn run_app(window: Window, lifecycle: Lifecycle) -> windows::core::Result<()> {
        // Start a fresh log each launch, so a stale one cannot be mistaken
        // for this run's.
        if let Some(local) = horizon_platform_uwp::local_folder() {
            let _ = std::fs::remove_file(local.join("smoke.log"));
        }
        log("start");
        match render(window, lifecycle) {
            Ok(()) => {
                log("clean exit");
                Ok(())
            }
            Err(e) => {
                log(&format!("FAILED: {e}"));
                Err(e)
            }
        }
    }

    fn render(mut window: Window, lifecycle: Lifecycle) -> windows::core::Result<()> {
        let (width, height) = window.inner_size();
        log(&format!(
            "window {width}x{height} scale {:.2}",
            window.scale_factor()
        ));

        // DX12 only: it is the one backend an Xbox has.
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::DX12;
        let instance = wgpu::Instance::new(descriptor);
        log("instance ok");

        // The safe `create_surface` wants a target that is Send + Sync. A
        // CoreWindow is apartment-bound and deliberately neither, so the raw
        // handles go in directly. Safety: `window` outlives `surface`, and
        // both are dropped at the end of this function.
        let surface = unsafe {
            let raw_window_handle = window
                .raw_window_handle()
                .map_err(|e| err(&format!("window handle: {e}")))?;
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(window.raw_display_handle()),
                raw_window_handle,
            })
        }
        .map_err(|e| err(&format!("create_surface: {e}")))?;
        log("surface ok");

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            ..Default::default()
        }))
        .map_err(|e| err(&format!("request_adapter: {e}")))?;
        log(&format!("adapter: {}", adapter.get_info().name));

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("smoke"),
            ..Default::default()
        }))
        .map_err(|e| err(&format!("request_device: {e}")))?;
        log("device ok");

        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats[0];
        let mut config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: caps.alpha_modes[0],
            view_formats: Vec::new(),
            desired_maximum_frame_latency: 2,
            color_space: wgpu::SurfaceColorSpace::default(),
        };
        surface.configure(&device, &config);
        log(&format!("configured {format:?} {width}x{height}"));

        let started = Instant::now();
        let mut frames: u64 = 0;
        let mut last_report = Instant::now();
        let mut quit = false;

        while !quit {
            window.pump_events(|event| match event {
                Event::Resized { width, height } if width > 0 && height > 0 => {
                    config.width = width;
                    config.height = height;
                    surface.configure(&device, &config);
                    log(&format!("resized {width}x{height}"));
                }
                // Menu / Start closes it, so the app can be left without
                // going back to Dev Home.
                Event::KeyDown(key) if key.0 == VK_GAMEPAD_MENU || key.0 == VK_ESCAPE => {
                    quit = true;
                }
                Event::CloseRequested => quit = true,
                Event::Focused(focused) => log(&format!("focus {focused}")),
                _ => {}
            })?;

            if lifecycle.take_suspending().is_some() {
                // The guard drops here. A real game would save first; this
                // only records that the notification arrived, which is the
                // thing worth proving.
                log("suspending");
            }
            if lifecycle.take_resumed() {
                log("resumed");
                surface.configure(&device, &config);
            }

            let frame = match surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(frame) => frame,
                // Suboptimal still gives a usable frame; draw it and let the
                // next resize sort the configuration out.
                wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
                wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                    continue;
                }
                other => {
                    // Outdated, Lost or Validation: reconfigure and try again.
                    log(&format!("acquire: {other:?}, reconfiguring"));
                    surface.configure(&device, &config);
                    continue;
                }
            };
            let view = frame.texture.create_view(&Default::default());
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                // Blue to purple, so a still photograph of the screen still
                // shows it is running rather than frozen.
                let t = started.elapsed().as_secs_f64();
                let r = 0.25 + 0.25 * (t * 0.7).sin();
                encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("clear"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r,
                                g: 0.08,
                                b: 0.45,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
            }
            queue.submit([encoder.finish()]);
            queue.present(frame);

            frames += 1;
            if last_report.elapsed() >= Duration::from_secs(5) {
                let mut line = String::new();
                let _ = write!(
                    line,
                    "{frames} frames, {:.1} fps",
                    frames as f64 / started.elapsed().as_secs_f64()
                );
                log(&line);
                last_report = Instant::now();
            }
        }
        Ok(())
    }

    /// `VK_GAMEPAD_MENU` and `VK_ESCAPE`: CoreWindow reports gamepad buttons
    /// as virtual keys, so the Menu button arrives as an ordinary KeyDown.
    const VK_GAMEPAD_MENU: u32 = 0xC3;
    const VK_ESCAPE: u32 = 0x1B;

    fn err(message: &str) -> windows::core::Error {
        windows::core::Error::new(windows::Win32::Foundation::E_FAIL, message)
    }
}
