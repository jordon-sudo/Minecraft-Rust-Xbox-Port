# Phase 0 — Feasibility report: Horizon on Xbox Dev Mode (UWP)

Baseline: horizon-rust/horizon @ `c332740d319c86c1fc85dbd047e90d4982447eb3`, imported unmodified.

> **This was the Phase 0 report; work has since started.** The findings below
> still stand, but three of the blockers have been dealt with. Current state:

| Blocker from this report | Now |
|---|---|
| wgpu-hal DX12 takes only `RawWindowHandle::Win32` | **Fixed.** `SurfaceTarget::CoreWindow` goes through `CreateSwapChainForCoreWindow`. Compiles for the UWP target. |
| winit cannot open a window in a UWP process | **Replaced** by `horizon-platform-uwp` (CoreApplication, CoreWindow, events, suspend/resume, paths). Compiles. **Not yet wired into the client.** |
| cpal cannot reach the default device | **Replaced** by `horizon_platform_uwp::AudioOutput`, via `ActivateAudioInterfaceAsync`. Compiles. **Not yet wired into `audio.rs`.** |
| gilrs on UWP: unverified | **Verified.** `gilrs 0.11.2` compiles clean for `x86_64-uwp-windows-msvc`, on its Windows.Gaming.Input backend. |
| `ring`, `libmimalloc-sys` on UWP | **Still unverified.** Their build scripts need an MSVC C compiler, which this Linux host has not. Must be checked on Windows. |

**The game does not run on Xbox yet.** The platform layer exists and compiles,
but `game.rs` still starts winit's event loop, so nothing calls it. That
wiring is the next step, and it needs a Windows machine: `horizon-client`
cannot be compiled for the UWP target on Linux at all, so none of it could be
verified here.

Nothing in this port has ever been packed, signed, installed or run on a
console.

---

## 1. Architecture summary

What owns what, from `CLAUDE.md`, the crate manifests and the source tree.

| Concern | Owner | Notes |
|---|---|---|
| **Windowing** | `winit 0.30.13`, used directly by `horizon-client` | `game.rs` implements `ApplicationHandler for Game` (`window_event`, `about_to_wait`). Window options in `horizon-client-input/src/window.rs`. |
| **Game loop** | `crates/horizon-client/src/game.rs` | `fn run`, `fn tick`, `fn tick_world`, `fn render`. `struct Game` (~290 fields) with ~180 `impl Game<'_>` files under `game/`. Driven by winit's `ApplicationHandler`. |
| **Rendering** | `horizon-client-render` (`gfx.rs`, `gfx/`, `*.wgsl`) on `wgpu 30.0.1` | Backends by platform: Vulkan/Metal/GLES by default, **`dx12` already enabled on `cfg(windows)`**. `naga 30.0.1` with `hlsl-out`. |
| **Input** | `horizon-client-input` (`keybinds.rs`, `gamepad.rs`, `mouse.rs`, `keymapping.rs`), `game/pad.rs`, `game/pad_menu.rs` | Controllers via `gilrs 0.11` behind the `controllers` feature. On-screen keyboard **already exists**: `crates/horizon-client/src/osk.rs`. |
| **Audio** | `horizon-client-render` (`audio.rs` mixer, `sound_engine.rs`) on `cpal 0.16` + `symphonia` | Behind the `audio` feature. |
| **Filesystem paths** | `horizon-client-render/src/logging.rs` resolves the game home | `--home` > `$HORIZON_HOME` > platform data dir. Also `--game-dir`, `--player-dir`, `--resources`. Saves/options via `save.rs`, `options_file.rs`. |
| **Networking** | `horizon-protocol` (protocol 777), `net.rs` (network thread), `lan/`, `horizon-friends`/`horizon-relay` (invite codes) | HTTPS via `ureq 2` + `rustls` + `ring`. |
| **Account** | `ureq` to Minecraft services; `docs/microsoft-account.md` | Online-mode session join. |
| **Prepared resources** | `horizon-prepare` builds `.local/horizon-client/26.3` from the player's own Minecraft install | Never shipped; never committed. |

Notable: single player is **not** an in-frame world tick — `game/integrated.rs` runs a second `Game` on a server thread and the window joins it as a client. Anything touching save-on-suspend must account for that.

---

## 2. Method, and the limits of what I could verify here

This session runs on **Linux**. That bounds what is provable:

- `cargo tree --target x86_64-uwp-windows-msvc` — full dependency resolution. **Reliable.**
- `cargo +nightly build -Zbuild-std --target x86_64-uwp-windows-msvc` — runs until a crate needs an **MSVC C compiler**. **Partial.**
- Crate source inspection for UWP-forbidden APIs. **Reliable.**
- Anything requiring link, deploy or run on hardware. **Not possible here.**

So: compile-level findings below are evidence-backed; runtime findings are reasoned from API availability and marked as such. **Completing the per-crate triage needs a Windows host with VS Build Tools.**

---

## 3. Target platform facts — checked

| Your assumption | Finding |
|---|---|
| Dev Mode runs UWP, deployed via Device Portal; set type to "Game" | **Holds.** Microsoft still documents Device Portal sideloading for Xbox. **But** Microsoft now states *"UWP based games are no longer accepted in the Xbox Store."* That is a Store policy, not Dev Mode — sideloading is unaffected, but this is a deprecated path with no future. |
| D3D12 only | **Holds.** And `horizon-client` already enables wgpu's `dx12` feature on Windows, so the backend is in the tree. |
| CoreWindow, not HWND; winit likely unsupported | **Confirmed, with a twist.** `winit 0.30.13` **compiles cleanly** for `x86_64-uwp-windows-msvc` — I built it. It is a **runtime** blocker, not a build blocker: a UWP process cannot create an HWND top-level window. Do not expect a compile error to tell you it is wrong. |
| Tier 3, needs nightly + `-Zbuild-std` | **Holds**, and better than expected: `core`, `compiler_builtins` and `std` **all built clean** on current nightly. The long-standing breakage ([rust#138257](https://github.com/rust-lang/rust/issues/138257), `NtOpenFile`) was **fixed in March 2025**. |
| Windows.Gaming.Input for gamepads | **Holds — and largely already done.** `gilrs-core 0.6.8` ships a `windows_wgi` backend and its **default feature is `wgi`**, built on `Gaming_Input`. Phase 2 may need far less work than budgeted. |
| Sandboxed filesystem | **Holds.** Package dir read-only; LocalFolder/TemporaryFolder writable. |
| Manifest network capabilities | **Holds.** |
| Memory tighter than PC | **Holds, numbers unconfirmed.** I could not find an authoritative current figure for *Dev Mode UWP* budgets. Third-party reports disagree (older 1 GB app / 5 GB game vs. a 2025 claim of ~5 GB Series S / ~8 GB Series X). **Measure on your console; do not design to a number from the internet.** Xbox also suspends or terminates an app that fails to shed memory within **2 seconds** of the limit-changed event. |

---

## 4. Dependency triage

`cargo tree` for `x86_64-uwp-windows-msvc` and `x86_64-pc-windows-msvc` resolve to **273 crates each — byte-identical sets**. Nothing in the tree cfg's itself out for UWP. Every Win32-dependent crate comes along and must be gated by *us*.

### Hard blockers

| Crate | Problem | Evidence | Fix |
|---|---|---|---|
| `winit 0.30.13` | Compiles; cannot create a top-level window in a UWP process. | Built it; Win32-only backend. | **Replace on UWP.** CoreApplication/CoreWindow loop via the `windows` crate behind cfg. |
| `wgpu-hal 30.0.1` | DX12 `create_surface` matches **only** `RawWindowHandle::Win32`; everything else hits `_ => Err("window handle {:?} is not a Win32 handle")`. It also `assert!`s the display handle is `Windows(_)` — a **panic**, not an error. `SurfaceTargetUnsafe` offers only `RawHandle` and `Drm`; no CoreWindow escape hatch. | wgpu-hal source. | **Patch the existing vendored fork.** Horizon *already* vendors `third_party/wgpu-hal` via `[patch.crates-io]`, so this is editing a fork you already own — not forking a new dependency. Add a `WinRt` arm calling `CreateSwapChainForCoreWindow`. `raw-window-handle 0.6.2` already has the `WinRt` variant. |
| `cpal 0.16` | WASAPI backend enumerates via `CoCreateInstance::<IMMDeviceEnumerator>(&MMDeviceEnumerator, …)`, which UWP forbids. No `ActivateAudioInterfaceAsync` anywhere in the crate. | cpal source (`host/wasapi/device.rs:836`). | Compiles, fails at runtime. Needs a UWP audio backend (`ActivateAudioInterfaceAsync` + `MediaDevice.GetDefaultAudioRenderId`). |

### Likely runtime blockers (compile fine, fail on console)

| Crate | Problem |
|---|---|
| `arboard 3.6.1` | Win32 clipboard. **Non-optional** dep of `horizon-client-input`. UWP needs `Windows.ApplicationModel.DataTransfer`. |
| `open 5.4.5` | `ShellExecute`. Win32-only. Feature-gate off. |
| `horizon-threads` | `windows-sys` with `Win32_UI_WindowsAndMessaging`, `Win32_Graphics_Gdi`, `Win32_System_ProcessStatus`, `Win32_System_Console`. Much of this is outside the UWP API surface. |
| `horizon-narrator` | SAPI `SpVoice` over COM. Disable on Xbox v1. |

### Could not be verified from Linux

`ring 0.17.14` and `libmimalloc-sys` both halt the build — their build scripts need an **MSVC C compiler**, absent here. This is a *host* limitation, not evidence of a UWP problem. Both are plausible UWP risks (ring's asm, mimalloc's OS hooks) and **must be re-run on Windows**. `ring` is non-optional: it arrives through `ureq` → `rustls`, so TLS, account login and multiplayer all sit behind it.

### Already fine

`gilrs` (WGI by default), `wgpu`/`naga` DX12 + `hlsl-out`, `horizon-client-input` — **compiled clean for the UWP target**, winit included.

---

## 5. The winit coupling is narrower than the file count suggests

99 files reference winit, which looks alarming. The breakdown does not:

| Import | Count |
|---|---|
| `winit::keyboard` | **151 refs / 53 imports** |
| `winit::event` | 22 |
| `winit::window` | 20 |
| `winit::dpi` | 17 |
| `winit::event_loop` | **2** |

The overwhelming majority is `winit::keyboard::KeyCode` used as a **plain data type**, not windowing behaviour. A platform shim that re-exports those enums on desktop and mirrors them on UWP covers most of it. Genuine windowing coupling is concentrated in a handful of files: `game.rs` (`ApplicationHandler`), `horizon-client-input/src/window.rs`, `game/split/windows.rs`, `horizon-hdr/src/wayland.rs`, `horizon-threads/src/boot_paint.rs`.

**This is the single most encouraging finding in the report** — it means Phase 1 can plausibly meet your "don't fork the game loop" requirement.

---

## 6. Recommended v1 scope cuts

Disable on Xbox: native Rust mods built at runtime (the launcher compiles them — impossible in the sandbox), `horizon-ctl`/MCP/agent control, dedicated-server hosting, the narrator, DualSense HID output, telemetry self-update, distant terrain/bluemap until memory is measured.

Keep: single player, save/load, multiplayer client, LAN join, content mods (`.hmod`, no code).

---

## 7. Effort estimate

Assumes one engineer who can build and deploy on Windows hardware. **Large and uncertain.**

| Phase | Estimate | Confidence |
|---|---|---|
| 0 — finish triage on Windows | 2–4 days | High |
| 1 — CoreWindow loop + wgpu-hal surface | **3–5 weeks** | **Low** — riskiest phase |
| 2 — Input (gilrs/WGI already there) + UI navigation | 2–3 weeks | Medium |
| 3 — Paths, LocalFolder, resource-missing screen | 1–1.5 weeks | High |
| 4 — UWP audio backend, device-code login, multiplayer | 2–4 weeks | Low–Medium |
| 5 — Packaging, xtask, DEPLOY_XBOX.md | 1–2 weeks | Medium |
| 6 — Performance and memory | 2–4 weeks | Low |

**Realistic total: 3–5 months**, dominated by Phase 1 and by iteration speed on a console you must deploy to for every test.

## 8. Riskiest unknowns, worst first

1. **Does a Rust UWP app run on Xbox at all?** An open winit issue ([#2506](https://github.com/rust-windowing/winit/issues/2506)) reports a trivial Rust UWP app that ran on Windows but **would not run on Xbox Series S**. If that reproduces, the project stops here. **Prove this first with a hello-world, before any porting work.**
2. **Memory.** Unknown real budget, 2-second eviction deadline, and Horizon streams chunks aggressively.
3. **`ring` on UWP.** Unverified and non-optional; blocks TLS, login and multiplayer.
4. **D3D12 feature level** on Xbox UWP vs. what Horizon's shaders assume.
5. **UWP is a dead-end platform** by Microsoft's own statement. Fine for personal sideloading; no future.

## 9. Recommendation

**Feasible, not cheap, and gated on unknown #1.** Nothing I found is fatal, and two findings (gilrs already on WGI; wgpu-hal already vendored) are materially better than the brief assumed. But the winit-on-Xbox report means Phase 1 could collapse on contact.

**Proposed next step before Phase 1: a throwaway spike** — a Rust UWP hello-world that opens a CoreWindow and clears a DX12 swapchain, packaged and sideloaded to your console. Days, not weeks, and it retires the two biggest risks at once.

## 10. Decisions I need from you

1. **Run the spike first**, or go straight into Phase 1?
2. **`third_party/wgpu-hal` patch** — you asked to be consulted before forking a major dependency. This is editing a fork Horizon *already maintains*, which I read as inside your rules, but confirm.
3. **A Windows machine** to finish the triage and to build/package. Do you have one?
4. Confirm the §6 cut list.
