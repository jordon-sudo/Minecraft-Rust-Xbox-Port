//! Audio output on Xbox.
//!
//! `cpal`'s WASAPI backend reaches the default device with
//! `CoCreateInstance(MMDeviceEnumerator)`, which a UWP app is not allowed to
//! do: the class is not registered in the sandbox. UWP's route is
//! `ActivateAudioInterfaceAsync` against the device id
//! `Windows.Media.Devices` hands out, so this module opens the stream that
//! way and then drives an ordinary WASAPI shared-mode render loop.
//!
//! The caller supplies a fill callback and gets back an [`AudioOutput`];
//! audio plays until that value is dropped.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use windows::{
    Media::Devices::{AudioDeviceRole, MediaDevice},
    Win32::{
        Foundation::{HANDLE, WAIT_OBJECT_0},
        Media::Audio::{
            AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation,
            IActivateAudioInterfaceCompletionHandler,
            IActivateAudioInterfaceCompletionHandler_Impl, IAudioClient, IAudioRenderClient,
            WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
        },
        System::{
            Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize},
            Threading::{CreateEventW, INFINITE, SetEvent, WaitForSingleObject},
        },
    },
    core::{HSTRING, Interface, Result, implement},
};

/// How the mixer is asked for samples: interleaved `f32`, `channels` per
/// frame, at `sample_rate`.
pub trait Fill: Send + 'static {
    fn fill(&mut self, out: &mut [f32], channels: usize, sample_rate: u32);
}

impl<F> Fill for F
where
    F: FnMut(&mut [f32], usize, u32) + Send + 'static,
{
    fn fill(&mut self, out: &mut [f32], channels: usize, sample_rate: u32) {
        self(out, channels, sample_rate)
    }
}

/// A playing output stream. Dropping it stops playback and joins the thread.
pub struct AudioOutput {
    running: Arc<AtomicBool>,
    wake: SafeHandle,
    thread: Option<std::thread::JoinHandle<()>>,
    channels: usize,
    sample_rate: u32,
}

impl AudioOutput {
    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Opens the default render device and starts playing.
    pub fn new(fill: impl Fill) -> Result<Self> {
        // The format is read on this thread so the caller learns the rate and
        // channel count up front; the client itself is opened again on the
        // render thread, which is where it must live.
        let (channels, sample_rate) = probe_format()?;

        let running = Arc::new(AtomicBool::new(true));
        let wake = SafeHandle(unsafe { CreateEventW(None, false, false, None) }?);

        let thread_running = running.clone();
        let thread_wake = wake.clone();
        let thread = std::thread::Builder::new()
            .name("horizon-audio-uwp".to_owned())
            .spawn(move || {
                if let Err(e) = render_loop(fill, &thread_running, &thread_wake) {
                    log::error!("Xbox audio output stopped: {e}");
                }
            })
            .map_err(|e| {
                windows::core::Error::new(windows::Win32::Foundation::E_FAIL, e.to_string())
            })?;

        Ok(Self {
            running,
            wake,
            thread: Some(thread),
            channels,
            sample_rate,
        })
    }
}

impl Drop for AudioOutput {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        // The render thread parks on the event, so nudge it or the join waits
        // for a whole buffer period.
        let _ = unsafe { SetEvent(self.wake.0) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// A handle shared with the render thread. WASAPI event handles are safe to
/// signal and wait on from any thread.
#[derive(Clone)]
struct SafeHandle(HANDLE);
unsafe impl Send for SafeHandle {}
unsafe impl Sync for SafeHandle {}

impl Drop for SafeHandle {
    fn drop(&mut self) {
        // Cloned handles share one kernel object; closing it here while the
        // thread still waits would be a use-after-free, so the handle is
        // deliberately leaked for the life of the process. There is one per
        // audio device open, and Horizon opens one.
    }
}

/// Opens the default render client, synchronously.
///
/// `ActivateAudioInterfaceAsync` completes on a system thread, so this blocks
/// on an event the completion handler signals.
fn activate_audio_client() -> Result<IAudioClient> {
    let id: HSTRING = MediaDevice::GetDefaultAudioRenderId(AudioDeviceRole::Default)?;
    let done = SafeHandle(unsafe { CreateEventW(None, true, false, None) }?);
    let handler: IActivateAudioInterfaceCompletionHandler =
        Completion { done: done.clone() }.into();

    let operation: IActivateAudioInterfaceAsyncOperation =
        unsafe { ActivateAudioInterfaceAsync(&id, &IAudioClient::IID, None, &handler) }?;

    if unsafe { WaitForSingleObject(done.0, INFINITE) } != WAIT_OBJECT_0 {
        return Err(windows::core::Error::new(
            windows::Win32::Foundation::E_FAIL,
            "waiting for ActivateAudioInterfaceAsync failed",
        ));
    }

    let mut hr = windows::core::HRESULT(0);
    let mut unknown = None;
    unsafe { operation.GetActivateResult(&mut hr, &mut unknown) }?;
    hr.ok()?;
    unknown
        .ok_or_else(|| {
            windows::core::Error::new(
                windows::Win32::Foundation::E_FAIL,
                "no audio interface was activated",
            )
        })?
        .cast::<IAudioClient>()
}

#[implement(IActivateAudioInterfaceCompletionHandler)]
struct Completion {
    done: SafeHandle,
}

impl IActivateAudioInterfaceCompletionHandler_Impl for Completion_Impl {
    fn ActivateCompleted(
        &self,
        _operation: windows_core::Ref<IActivateAudioInterfaceAsyncOperation>,
    ) -> Result<()> {
        unsafe { SetEvent(self.done.0) }
    }
}

/// The mix format's channel count and sample rate.
fn probe_format() -> Result<(usize, u32)> {
    let _com = ComGuard::new()?;
    let client = activate_audio_client()?;
    let format = unsafe { client.GetMixFormat() }?;
    let (channels, rate) = unsafe { ((*format).nChannels as usize, (*format).nSamplesPerSec) };
    unsafe { CoTaskMemFree(Some(format as *const _)) };
    Ok((channels, rate))
}

fn render_loop(mut fill: impl Fill, running: &AtomicBool, wake: &SafeHandle) -> Result<()> {
    let _com = ComGuard::new()?;
    let client = activate_audio_client()?;
    let format = unsafe { client.GetMixFormat() }?;
    let (channels, rate, bits, is_float) = unsafe { describe(format) };

    // Shared mode with AUTOCONVERTPCM lets the engine resample if the mixer's
    // rate ever differs from the device's, rather than failing to open.
    let result = unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            0,
            0,
            format,
            None,
        )
    };
    unsafe { CoTaskMemFree(Some(format as *const _)) };
    result?;

    unsafe { client.SetEventHandle(wake.0) }?;
    let render: IAudioRenderClient = unsafe { client.GetService() }?;
    let buffer_frames = unsafe { client.GetBufferSize() }?;
    unsafe { client.Start() }?;

    let mut scratch: Vec<f32> = Vec::new();
    while running.load(Ordering::SeqCst) {
        if unsafe { WaitForSingleObject(wake.0, INFINITE) } != WAIT_OBJECT_0 {
            break;
        }
        if !running.load(Ordering::SeqCst) {
            break;
        }
        let padding = unsafe { client.GetCurrentPadding() }?;
        let frames = buffer_frames.saturating_sub(padding);
        if frames == 0 {
            continue;
        }

        let data = unsafe { render.GetBuffer(frames) }?;
        let samples = frames as usize * channels;
        scratch.clear();
        scratch.resize(samples, 0.0);
        fill.fill(&mut scratch, channels, rate);
        unsafe { write_samples(data, &scratch, bits, is_float) };
        unsafe { render.ReleaseBuffer(frames, 0) }?;
    }

    let _ = unsafe { client.Stop() };
    Ok(())
}

/// Channels, sample rate, bits per sample, and whether samples are floats.
///
/// A shared-mode mix format is normally 32-bit float, but it arrives as
/// `WAVEFORMATEXTENSIBLE` whose tag is `EXTENSIBLE` rather than the format
/// itself, so the subformat decides.
unsafe fn describe(format: *const WAVEFORMATEX) -> (usize, u32, u16, bool) {
    let base = unsafe { &*format };
    let bits = base.wBitsPerSample;
    let is_float = if base.wFormatTag == WAVE_FORMAT_IEEE_FLOAT {
        true
    } else if base.wFormatTag == WAVE_FORMAT_PCM {
        false
    } else {
        // EXTENSIBLE: the subformat GUID says which it really is.
        // WAVEFORMATEXTENSIBLE is packed, so the field cannot be borrowed;
        // it has to be read out unaligned.
        let sub = unsafe {
            std::ptr::read_unaligned(
                &raw const (*(format as *const WAVEFORMATEXTENSIBLE)).SubFormat,
            )
        };
        sub == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT
    };
    (base.nChannels as usize, base.nSamplesPerSec, bits, is_float)
}

/// The two wave format tags this module cares about. They live in
/// `Win32::Media::Multimedia`, which is a large feature to enable for two
/// numbers that are fixed by the format.
const WAVE_FORMAT_PCM: u16 = 1;
const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;

/// `KSDATAFORMAT_SUBTYPE_IEEE_FLOAT`, spelled out so this module does not
/// need the whole KernelStreaming feature for one constant.
const KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: windows::core::GUID =
    windows::core::GUID::from_u128(0x0000_0003_0000_0010_8000_00aa_00389b71);

unsafe fn write_samples(out: *mut u8, samples: &[f32], bits: u16, is_float: bool) {
    match (is_float, bits) {
        (true, 32) => {
            let out = out as *mut f32;
            for (i, &s) in samples.iter().enumerate() {
                unsafe { *out.add(i) = s };
            }
        }
        (false, 16) => {
            let out = out as *mut i16;
            for (i, &s) in samples.iter().enumerate() {
                let v = (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
                unsafe { *out.add(i) = v };
            }
        }
        (false, 32) => {
            let out = out as *mut i32;
            for (i, &s) in samples.iter().enumerate() {
                let v = (f64::from(s.clamp(-1.0, 1.0)) * f64::from(i32::MAX)) as i32;
                unsafe { *out.add(i) = v };
            }
        }
        _ => {
            // An unexpected format: silence beats noise at full volume.
            let bytes = samples.len() * (bits as usize / 8).max(1);
            unsafe { std::ptr::write_bytes(out, 0, bytes) };
        }
    }
}

/// COM on this thread, for as long as the guard lives.
struct ComGuard;

impl ComGuard {
    fn new() -> Result<Self> {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok()?;
        Ok(Self)
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}
