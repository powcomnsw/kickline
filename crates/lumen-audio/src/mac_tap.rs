//! Capture an app's audio directly (macOS 14.2+ Core Audio process taps).
//!
//! This listens to rekordbox's own output, so there is no BlackHole, no
//! Multi-Output Device and no extra buffering, and you still hear the music
//! through your controller as normal.
//!
//! How it works: create a private, unmuted tap on the app's audio process,
//! wrap it in a private aggregate device, and read that device's input with
//! an IOProc. The first time, macOS asks for permission ("System Audio
//! Recording"); if refused, the tap delivers silence, which the probe detects.
//!
//! On other platforms the same API exists but reports "not supported".

use crate::analyzer::Wake;
use crate::input::AudioInput;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppInfo {
    pub pid: i32,
    pub name: String,
    pub bundle_id: String,
    /// The app is currently playing audio.
    pub playing: bool,
}

impl AppInfo {
    pub fn is_rekordbox(&self) -> bool {
        let n = self.name.to_ascii_lowercase();
        let b = self.bundle_id.to_ascii_lowercase();
        n.contains("rekordbox") || b.contains("rekordbox")
    }

    pub fn label(&self) -> String {
        let name = if self.name.is_empty() { &self.bundle_id } else { &self.name };
        if self.playing {
            format!("{name} (direct) ●")
        } else {
            format!("{name} (direct)")
        }
    }
}

/// Pick rekordbox from a list of apps, preferring one that is playing.
pub fn find_rekordbox(apps: &[AppInfo]) -> Option<&AppInfo> {
    apps.iter().filter(|a| a.is_rekordbox()).max_by_key(|a| a.playing)
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::*;

    pub fn supported() -> Result<(), String> {
        Err("Capturing an app directly needs macOS 14.2 or newer.".into())
    }

    pub fn list_audio_apps() -> Vec<AppInfo> {
        Vec::new()
    }

    pub fn start(_app: &AppInfo, _wake: Option<Arc<Wake>>) -> Result<AudioInput, String> {
        Err(supported().unwrap_err())
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use crate::input::{AudioFeed, FeedKind};
    use objc2::rc::Retained;
    use objc2::AllocAnyThread;
    use objc2_core_audio::{
        kAudioAggregateDeviceIsPrivateKey, kAudioAggregateDeviceIsStackedKey, kAudioAggregateDeviceMainSubDeviceKey,
        kAudioAggregateDeviceNameKey, kAudioAggregateDeviceSubDeviceListKey, kAudioAggregateDeviceTapAutoStartKey,
        kAudioAggregateDeviceTapListKey, kAudioAggregateDeviceUIDKey, kAudioDevicePropertyDeviceUID,
        kAudioHardwarePropertyDefaultOutputDevice, kAudioHardwarePropertyProcessObjectList, kAudioObjectPropertyElementMain,
        kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject, kAudioProcessPropertyBundleID,
        kAudioProcessPropertyIsRunningOutput, kAudioProcessPropertyPID, kAudioSubDeviceUIDKey,
        kAudioSubTapDriftCompensationKey, kAudioSubTapUIDKey, kAudioTapPropertyFormat, AudioDeviceCreateIOProcID,
        AudioDeviceDestroyIOProcID, AudioDeviceIOProcID, AudioDeviceStart, AudioDeviceStop,
        AudioHardwareCreateAggregateDevice, AudioHardwareDestroyAggregateDevice, AudioObjectGetPropertyData,
        AudioObjectGetPropertyDataSize, AudioObjectID, AudioObjectPropertyAddress, CATapDescription, CATapMuteBehavior,
    };
    use objc2_core_audio_types::{AudioBufferList, AudioStreamBasicDescription, AudioTimeStamp};
    use objc2_core_foundation::CFDictionary;
    use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSObject, NSString, NSUUID};
    use std::ffi::{c_void, CStr};
    use std::ptr::{self, NonNull};
    use std::sync::Mutex;

    type OSStatus = i32;
    type CreateTapFn = unsafe extern "C-unwind" fn(Option<&CATapDescription>, *mut AudioObjectID) -> OSStatus;
    type DestroyTapFn = unsafe extern "C-unwind" fn(AudioObjectID) -> OSStatus;

    const SYSTEM: AudioObjectID = kAudioObjectSystemObject as AudioObjectID;

    /// Look the tap functions up at run time, so Kickline still launches on
    /// macOS versions older than 14.2 (it just can't offer direct capture).
    fn sym<T: Copy>(name: &CStr) -> Option<T> {
        // SAFETY: dlsym with RTLD_DEFAULT and a NUL-terminated name.
        let p = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
        if p.is_null() {
            None
        } else {
            // SAFETY: T is the matching function-pointer type for this symbol.
            Some(unsafe { std::mem::transmute_copy::<*mut c_void, T>(&p) })
        }
    }

    fn tap_fns() -> Option<(CreateTapFn, DestroyTapFn)> {
        Some((sym::<CreateTapFn>(c"AudioHardwareCreateProcessTap")?, sym::<DestroyTapFn>(c"AudioHardwareDestroyProcessTap")?))
    }

    pub fn supported() -> Result<(), String> {
        tap_fns().map(|_| ()).ok_or_else(|| "Capturing an app directly needs macOS 14.2 or newer.".to_string())
    }

    fn addr(sel: u32) -> AudioObjectPropertyAddress {
        AudioObjectPropertyAddress { mSelector: sel, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain }
    }

    /// Read a fixed-size property.
    unsafe fn get<T: Copy>(obj: AudioObjectID, sel: u32) -> Result<T, OSStatus> {
        let mut a = addr(sel);
        let mut size = std::mem::size_of::<T>() as u32;
        let mut v: T = std::mem::zeroed();
        let st = AudioObjectGetPropertyData(
            obj,
            NonNull::from(&mut a),
            0,
            ptr::null(),
            NonNull::from(&mut size),
            NonNull::from(&mut v).cast(),
        );
        if st == 0 {
            Ok(v)
        } else {
            Err(st)
        }
    }

    /// Read an array property.
    unsafe fn get_vec<T: Copy>(obj: AudioObjectID, sel: u32) -> Result<Vec<T>, OSStatus> {
        let mut a = addr(sel);
        let mut size = 0u32;
        let st = AudioObjectGetPropertyDataSize(obj, NonNull::from(&mut a), 0, ptr::null(), NonNull::from(&mut size));
        if st != 0 {
            return Err(st);
        }
        let n = size as usize / std::mem::size_of::<T>();
        let mut v: Vec<T> = Vec::with_capacity(n);
        if n == 0 {
            return Ok(v);
        }
        let st = AudioObjectGetPropertyData(
            obj,
            NonNull::from(&mut a),
            0,
            ptr::null(),
            NonNull::from(&mut size),
            NonNull::new_unchecked(v.as_mut_ptr()).cast(),
        );
        if st != 0 {
            return Err(st);
        }
        v.set_len(size as usize / std::mem::size_of::<T>());
        Ok(v)
    }

    /// Read a CFString property (returned +1, so we take ownership).
    unsafe fn get_string(obj: AudioObjectID, sel: u32) -> Option<String> {
        let p: *mut NSString = get(obj, sel).ok()?;
        Retained::from_raw(p).map(|s| s.to_string())
    }

    fn process_name(pid: i32) -> String {
        let mut buf = [0u8; 256];
        // SAFETY: buffer is valid for its length.
        let n = unsafe { libc::proc_name(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
        if n > 0 {
            String::from_utf8_lossy(&buf[..n as usize]).to_string()
        } else {
            String::new()
        }
    }

    fn process_object(pid: i32) -> Option<AudioObjectID> {
        // SAFETY: plain property reads on the system object.
        let ids: Vec<AudioObjectID> = unsafe { get_vec(SYSTEM, kAudioHardwarePropertyProcessObjectList).ok()? };
        ids.into_iter().find(|&id| unsafe { get::<i32>(id, kAudioProcessPropertyPID) }.ok() == Some(pid))
    }

    pub fn list_audio_apps() -> Vec<AppInfo> {
        if supported().is_err() {
            return Vec::new();
        }
        let me = std::process::id() as i32;
        // SAFETY: plain property reads.
        let ids: Vec<AudioObjectID> = unsafe { get_vec(SYSTEM, kAudioHardwarePropertyProcessObjectList).unwrap_or_default() };
        let mut out: Vec<AppInfo> = ids
            .into_iter()
            .filter_map(|id| unsafe {
                let pid: i32 = get(id, kAudioProcessPropertyPID).ok()?;
                if pid == me {
                    return None;
                }
                let bundle_id = get_string(id, kAudioProcessPropertyBundleID).unwrap_or_default();
                let playing = get::<u32>(id, kAudioProcessPropertyIsRunningOutput).unwrap_or(0) != 0;
                Some(AppInfo { pid, name: process_name(pid), bundle_id, playing })
            })
            .filter(|a| !a.name.is_empty() || !a.bundle_id.is_empty())
            .collect();
        out.sort_by(|a, b| b.is_rekordbox().cmp(&a.is_rekordbox()).then(b.playing.cmp(&a.playing)).then(a.name.cmp(&b.name)));
        out
    }

    fn key(k: &CStr) -> Retained<NSString> {
        NSString::from_str(k.to_str().unwrap_or_default())
    }

    fn obj<T: objc2::Message>(v: Retained<T>) -> Retained<NSObject> {
        // SAFETY: every Foundation object is an NSObject.
        unsafe { Retained::cast_unchecked(v) }
    }

    fn dict(pairs: Vec<(&CStr, Retained<NSObject>)>) -> Retained<NSDictionary<NSString, NSObject>> {
        let keys: Vec<Retained<NSString>> = pairs.iter().map(|(k, _)| key(k)).collect();
        let key_refs: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
        let vals: Vec<Retained<NSObject>> = pairs.into_iter().map(|(_, v)| v).collect();
        NSDictionary::from_retained_objects(&key_refs, &vals)
    }

    struct TapCtx {
        feed: AudioFeed,
        mono: Mutex<Vec<f32>>,
    }

    /// Owns the tap, aggregate device and IOProc; tears them down on drop.
    struct TapHandle {
        destroy_tap: DestroyTapFn,
        tap: Option<AudioObjectID>,
        agg: Option<AudioObjectID>,
        proc_id: AudioDeviceIOProcID,
        started: bool,
        ctx: *mut TapCtx,
    }

    impl Drop for TapHandle {
        fn drop(&mut self) {
            // SAFETY: each id was created by us and is destroyed once.
            unsafe {
                if let Some(agg) = self.agg {
                    if self.started {
                        AudioDeviceStop(agg, self.proc_id);
                    }
                    if self.proc_id.is_some() {
                        AudioDeviceDestroyIOProcID(agg, self.proc_id);
                    }
                    AudioHardwareDestroyAggregateDevice(agg);
                }
                if let Some(tap) = self.tap {
                    (self.destroy_tap)(tap);
                }
                if !self.ctx.is_null() {
                    drop(Box::from_raw(self.ctx));
                }
            }
        }
    }

    unsafe extern "C-unwind" fn io_proc(
        _device: AudioObjectID,
        _now: NonNull<AudioTimeStamp>,
        input: NonNull<AudioBufferList>,
        _input_time: NonNull<AudioTimeStamp>,
        _output: NonNull<AudioBufferList>,
        _output_time: NonNull<AudioTimeStamp>,
        client: *mut c_void,
    ) -> OSStatus {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // SAFETY: client is the TapCtx we registered; buffers come from Core Audio.
            let ctx = unsafe { &*(client as *const TapCtx) };
            let abl = unsafe { input.as_ref() };
            let n = abl.mNumberBuffers as usize;
            if n == 0 {
                return;
            }
            let bufs = unsafe { std::slice::from_raw_parts(abl.mBuffers.as_ptr(), n) };
            let frames = bufs
                .iter()
                .map(|b| b.mDataByteSize as usize / 4 / (b.mNumberChannels.max(1) as usize))
                .min()
                .unwrap_or(0);
            let total_ch: usize = bufs.iter().map(|b| b.mNumberChannels.max(1) as usize).sum();
            let Ok(mut mono) = ctx.mono.try_lock() else { return };
            mono.clear();
            mono.resize(frames, 0.0);
            for b in bufs {
                if b.mData.is_null() {
                    continue;
                }
                let ch = b.mNumberChannels.max(1) as usize;
                let data = unsafe { std::slice::from_raw_parts(b.mData as *const f32, frames * ch) };
                for (f, m) in mono.iter_mut().enumerate() {
                    for c in 0..ch {
                        *m += data[f * ch + c];
                    }
                }
            }
            let k = 1.0 / total_ch.max(1) as f32;
            for m in mono.iter_mut() {
                *m *= k;
            }
            ctx.feed.health.deliver(ctx.feed.started, &ctx.feed.analyzer, &mono);
        }));
        0
    }

    pub fn start(app: &AppInfo, wake: Option<Arc<Wake>>) -> Result<AudioInput, String> {
        let (create_tap, destroy_tap) = tap_fns().ok_or_else(|| supported().unwrap_err())?;
        let process = process_object(app.pid).ok_or_else(|| format!("{} isn't running (or has no audio yet)", app.name))?;
        let mut h = TapHandle { destroy_tap, tap: None, agg: None, proc_id: None, started: false, ctx: ptr::null_mut() };

        // SAFETY: Core Audio / Foundation calls with valid arguments; ids are tracked in `h`.
        unsafe {
            // 1. The tap: an unmuted, private stereo mix of the app's output.
            let procs = NSArray::from_retained_slice(&[NSNumber::numberWithUnsignedInt(process)]);
            let desc = CATapDescription::initStereoMixdownOfProcesses(CATapDescription::alloc(), &procs);
            desc.setName(&NSString::from_str("Kickline"));
            desc.setPrivate(true);
            desc.setMuteBehavior(CATapMuteBehavior::Unmuted);
            let tap_uid = desc.UUID().UUIDString().to_string();
            let mut tap: AudioObjectID = 0;
            let st = create_tap(Some(&*desc), &mut tap);
            if st != 0 || tap == 0 {
                return Err(format!("couldn't create the audio tap (OSStatus {st})"));
            }
            h.tap = Some(tap);

            // 2. A private aggregate device around the tap, clocked by the current output.
            let out_dev: AudioObjectID =
                get(SYSTEM, kAudioHardwarePropertyDefaultOutputDevice).map_err(|e| format!("no output device (OSStatus {e})"))?;
            let out_uid = get_string(out_dev, kAudioDevicePropertyDeviceUID).ok_or("couldn't read the output device")?;
            let agg_uid = NSUUID::new().UUIDString();
            let sub = dict(vec![(kAudioSubDeviceUIDKey, obj(NSString::from_str(&out_uid)))]);
            let tap_entry = dict(vec![
                (kAudioSubTapDriftCompensationKey, obj(NSNumber::numberWithBool(true))),
                (kAudioSubTapUIDKey, obj(NSString::from_str(&tap_uid))),
            ]);
            let description = dict(vec![
                (kAudioAggregateDeviceNameKey, obj(NSString::from_str("Kickline rekordbox tap"))),
                (kAudioAggregateDeviceUIDKey, obj(agg_uid)),
                (kAudioAggregateDeviceMainSubDeviceKey, obj(NSString::from_str(&out_uid))),
                (kAudioAggregateDeviceIsPrivateKey, obj(NSNumber::numberWithBool(true))),
                (kAudioAggregateDeviceIsStackedKey, obj(NSNumber::numberWithBool(false))),
                (kAudioAggregateDeviceTapAutoStartKey, obj(NSNumber::numberWithBool(true))),
                (kAudioAggregateDeviceSubDeviceListKey, obj(NSArray::from_retained_slice(&[sub]))),
                (kAudioAggregateDeviceTapListKey, obj(NSArray::from_retained_slice(&[tap_entry]))),
            ]);
            // NSDictionary and CFDictionary are toll-free bridged.
            let cf: &CFDictionary = &*(Retained::as_ptr(&description) as *const CFDictionary);
            let mut agg: AudioObjectID = 0;
            let st = AudioHardwareCreateAggregateDevice(cf, NonNull::from(&mut agg));
            if st != 0 || agg == 0 {
                return Err(format!("couldn't create the capture device (OSStatus {st})"));
            }
            h.agg = Some(agg);

            // 3. Format of the tap, then start reading it.
            let fmt: AudioStreamBasicDescription = get(tap, kAudioTapPropertyFormat).map_err(|e| format!("tap format unavailable (OSStatus {e})"))?;
            let name = if app.name.is_empty() { app.bundle_id.clone() } else { app.name.clone() };
            let feed = AudioFeed::new(
                name,
                FeedKind::App,
                fmt.mSampleRate as u32,
                fmt.mChannelsPerFrame as u16,
                "f32".into(),
                wake,
            );
            h.ctx = Box::into_raw(Box::new(TapCtx { feed: feed.clone(), mono: Mutex::new(Vec::with_capacity(4096)) }));
            let mut proc_id: AudioDeviceIOProcID = None;
            let st = AudioDeviceCreateIOProcID(agg, Some(io_proc), h.ctx.cast(), NonNull::from(&mut proc_id));
            if st != 0 || proc_id.is_none() {
                return Err(format!("couldn't attach to the capture device (OSStatus {st})"));
            }
            h.proc_id = proc_id;
            let st = AudioDeviceStart(agg, proc_id);
            if st != 0 {
                return Err(format!("couldn't start capture (OSStatus {st})"));
            }
            h.started = true;
            Ok(AudioInput::new(feed, Box::new(h)))
        }
    }
}

/// Ok if this Mac can capture apps directly (macOS 14.2+).
pub fn supported() -> Result<(), String> {
    imp::supported()
}

/// Apps known to Core Audio, rekordbox first. Empty if unsupported.
pub fn list_audio_apps() -> Vec<AppInfo> {
    imp::list_audio_apps()
}

/// Start capturing one app's output.
pub fn start_app_capture(app: &AppInfo, wake: Option<Arc<Wake>>) -> Result<AudioInput, String> {
    imp::start(app, wake)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_rekordbox_preferring_playing() {
        let apps = vec![
            AppInfo { pid: 1, name: "Music".into(), bundle_id: "com.apple.Music".into(), playing: true },
            AppInfo { pid: 2, name: "rekordbox".into(), bundle_id: "com.pioneerdj.rekordboxdj".into(), playing: false },
            AppInfo { pid: 3, name: "rekordboxAgent".into(), bundle_id: "com.pioneerdj.rekordboxdj".into(), playing: true },
        ];
        assert_eq!(find_rekordbox(&apps).unwrap().pid, 3);
        assert!(find_rekordbox(&apps[..1]).is_none());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn unsupported_elsewhere() {
        assert!(supported().is_err());
        assert!(list_audio_apps().is_empty());
    }
}
