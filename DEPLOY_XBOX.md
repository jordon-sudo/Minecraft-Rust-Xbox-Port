# Deploying Horizon to an Xbox in Developer Mode

Horizon on Xbox is a sideloaded UWP app. This page covers turning Dev Mode on,
building and packaging the game, installing it, and getting the prepared
resources onto the console.

**Nothing here distributes Minecraft's files.** The package holds the game
binary only. You prepare the resources from your own copy of Minecraft on your
PC and copy them to the console yourself.

> **Status.** The platform layer, the DX12 CoreWindow surface and the
> packaging exist and compile. The port is not finished: see
> [docs/xbox-port-feasibility.md](docs/xbox-port-feasibility.md) for what is
> done and what is not, and read the **Known gaps** section below before you
> spend an evening on this.

---

## 1. Turn on Developer Mode

1. On the console, install **Dev Mode Activation** from the Store and run it.
   It shows an activation code.
2. On a PC, go to <https://partner.microsoft.com/xboxconfig/devices> and enter
   the code. You need a Partner Center account; the developer-console
   registration is a one-off fee.
3. The console reboots into Developer Mode. **Dev Home** is the shell you land
   in; switching back to retail mode is done from there.

## 2. Open Device Portal

1. In Dev Home, go to **Settings → Remote Access** (sometimes **Remote
   Management**) and turn remote access on.
2. Note the console's address. Device Portal is **HTTPS on port 11443**, so the
   URL is `https://<console-ip>:11443`.
3. The certificate is self-signed, so the browser warns. Accept it.
4. Set a username and password when prompted. Do not expose this port to the
   internet.

## 3. Build the game

The UWP target is **tier 3**: there are no prebuilt standard-library artifacts,
so it needs a nightly toolchain and `-Z build-std`. It must be built **on
Windows** — the MSVC linker and the Windows SDK are required, and some
dependencies (`ring`, `libmimalloc-sys`) compile C with them.

```powershell
rustup toolchain install nightly
rustup component add rust-src --toolchain nightly
```

```powershell
cargo +nightly build --release -Z build-std=std,panic_abort --target x86_64-uwp-windows-msvc -p horizon-client
```

## 4. Lay out and pack the package

```powershell
cargo run -p xtask -- package-xbox target\x86_64-uwp-windows-msvc\release\horizon-client.exe target\xbox "CN=<your cert subject>"
```

That writes the package layout: `AppxManifest.xml`, `horizon-client.exe` and
the icons. Packing and signing need the Windows SDK on `PATH`:

```powershell
makeappx pack /d target\xbox /p Horizon.msix
```

```powershell
signtool sign /fd SHA256 /a /f test.pfx /p <password> Horizon.msix
```

If you have no certificate yet, make a self-signed one. **The certificate's
subject and the manifest's `Publisher` must match exactly**, or installation
fails with a certificate error — this is the single most common mistake:

```powershell
New-SelfSignedCertificate -Type Custom -Subject "CN=Horizon Developer" -KeyUsage DigitalSignature -FriendlyName "Horizon Dev" -CertStoreLocation "Cert:\CurrentUser\My" -TextExtension @("2.5.29.37={text}1.3.6.1.5.5.7.3.3", "2.5.29.19={text}")
```

Export it to `test.pfx`, then pass the same subject to `package-xbox`.

## 5. Install it

1. In Device Portal, open **Home** (the app list) and choose **Add**.
2. Select `Horizon.msix` and, if the portal asks for dependencies, add them.
3. Install, and wait for it to appear in the list.

### Set the app type to Game

In Device Portal's app list, change Horizon's type from **App** to **Game**.

This matters: apps and games get different CPU, GPU and memory budgets, and
Horizon will not fit in the app budget. If the game dies on startup or while
loading terrain, check this first.

## 6. Copy the prepared resources

The package deliberately ships without them. On first launch Horizon looks in
its `LocalState\resources` folder and, finding nothing, tells you to do this.

On your PC, with your own copy of Minecraft 26.3:

```sh
cargo run --release -p horizon-prepare -- --prepare --server-jar ~/Downloads/server.jar
```

That writes `.local/horizon-client/26.3`. Then:

```sh
python3 tools/xbox_push_resources.py --host <console-ip> --user <user> --password <password>
```

See [docs/xbox-resources.md](docs/xbox-resources.md) for the manual Device
Portal route and for what the script does.

## 7. Run it

Launch Horizon from Dev Home. A controller is the only input device; there is
no mouse or keyboard by default.

---

## Known gaps

Read this before assuming a failure is your setup.

| Area | State |
|---|---|
| DX12 surface from CoreWindow | Implemented in `third_party/wgpu-hal`, compiles, **never run on hardware** |
| CoreWindow event loop, suspend/resume, paths | Implemented in `horizon-platform-uwp`, compiles, **not yet wired into the client** |
| Audio | **Broken.** `cpal`'s WASAPI backend uses `CoCreateInstance(MMDeviceEnumerator)`, which UWP forbids. Needs a UWP backend. |
| Clipboard (`arboard`), `open`, narrator | Win32-only; compile, will fail at runtime. To be disabled on Xbox. |
| Controller input | `gilrs` already defaults to a Windows.Gaming.Input backend, so this should mostly work once wired up. |
| Native Rust mods, horizon-ctl/MCP, server hosting | Not supported on Xbox; the launcher cannot compile code in the sandbox. |

## Common errors

**"The certificate is not trusted" / `0x800B0109`** — install the test
certificate into **Local Machine → Trusted People** on the PC, and confirm the
manifest `Publisher` matches the certificate subject character for character.

**"App installation failed" with no detail** — check Device Portal's own log
first. The usual causes are an architecture mismatch (the package must be x64)
and a `MinVersion` higher than the console's OS build.

**"Class not registered" on launch** — the manifest's `EntryPoint`. Horizon
registers no activatable WinRT class; it starts from `main` and calls
`CoreApplication::Run`. If activation fails here, that attribute is what to
change.

**Game starts, then dies loading a world** — almost always the memory budget.
Confirm the app type is **Game**, then lower the render distance.

**No resources screen on every launch** — the resources went to the wrong
folder. They belong in `LocalState\resources`, not `LocalState`.
