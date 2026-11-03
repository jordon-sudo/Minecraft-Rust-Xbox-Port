//! Where a UWP app may read and write.
//!
//! The package install folder is read-only, and the only writable storage is
//! what `ApplicationData` hands out. Horizon's own layout (saves, options,
//! logs, packs) is then built inside the local folder, so the existing save
//! and options code keeps working unchanged against ordinary paths.
//!
//! `LocalState` is also the folder Device Portal's file explorer shows, which
//! is how prepared resources get onto the console: see docs/xbox-resources.md.

use std::path::PathBuf;

use windows::Storage::ApplicationData;

/// The app's writable folders, resolved once at startup.
#[derive(Debug, Clone)]
pub struct Paths {
    local: PathBuf,
    temp: PathBuf,
}

impl Paths {
    pub fn new() -> windows::core::Result<Self> {
        let data = ApplicationData::Current()?;
        let local = PathBuf::from(data.LocalFolder()?.Path()?.to_string());
        let temp = PathBuf::from(data.TemporaryFolder()?.Path()?.to_string());
        Ok(Self { local, temp })
    }

    /// `LocalState`. Survives suspend, resume and app updates; visible in
    /// Device Portal.
    pub fn local(&self) -> &std::path::Path {
        &self.local
    }

    /// Scratch space the system may reclaim. Caches only: never saves.
    pub fn temp(&self) -> &std::path::Path {
        &self.temp
    }

    /// The game folder: saves, options.txt, packs, mods.
    pub fn game_dir(&self) -> PathBuf {
        self.local.join("horizon")
    }

    /// Logs and crash reports, the `--home` of the desktop build.
    pub fn home_dir(&self) -> PathBuf {
        self.local.join("horizon")
    }

    /// Where prepared resources are expected. The player copies the output of
    /// `horizon-prepare` here with Device Portal; nothing is bundled in the
    /// package, which must never carry Mojang's files.
    pub fn resources_dir(&self) -> PathBuf {
        self.local.join("resources")
    }

    /// Creates the folders the game writes into. The resources folder is not
    /// created: its absence is what tells the game to show the
    /// "copy your prepared resources" screen.
    pub fn create_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.game_dir())?;
        std::fs::create_dir_all(self.home_dir())?;
        Ok(())
    }
}
