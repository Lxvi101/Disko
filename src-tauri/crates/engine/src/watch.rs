//! FSEvents on the scan root: folder-level events, coalesced (~1 s), turned into `refresh` calls.
//!
//! The stream starts at the tree's event id when the tree has one (a fresh scan or a resumable
//! snapshot), so changes made while Disko wasn't looking — during the scan, or while the app was
//! closed — are replayed from the system's event history instead of rescanning.

use crate::ffi::*;
use crate::{Engine, EngineEvent};
use disko_core::WalkMatcher;
use std::ffi::{c_char, c_void, CStr};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(crate) struct Raw {
    pub path: String,
    pub flags: u32,
    pub id: u64,
}

/// Stops the watcher when dropped (or with [`WatchHandle::stop`]).
pub struct WatchHandle {
    stream: FSEventStreamRef,
    queue: DispatchQueue,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

unsafe impl Send for WatchHandle {}
unsafe impl Sync for WatchHandle {}

impl WatchHandle {
    pub fn stop(self) {}
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        unsafe {
            if !self.stream.is_null() {
                FSEventStreamStop(self.stream);
                FSEventStreamInvalidate(self.stream);
                FSEventStreamRelease(self.stream);
            }
            if !self.queue.is_null() {
                dispatch_release(self.queue);
            }
        }
        // The stream frees its sender (context release) once its last callback is done; the stop
        // flag ends the worker either way.
        self.stop.store(true, Ordering::Relaxed);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

extern "C" fn release_ctx(info: *const c_void) {
    unsafe { drop(Box::from_raw(info as *mut Sender<Vec<Raw>>)) };
}

extern "C" fn callback(_s: FSEventStreamRef, info: *mut c_void, n: usize, paths: *mut c_void, flags: *const u32, ids: *const u64) {
    let tx = unsafe { &*(info as *const Sender<Vec<Raw>>) };
    let paths = paths as *const *const c_char;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let p = unsafe { CStr::from_ptr(*paths.add(i)) }.to_string_lossy().into_owned();
        out.push(Raw { path: p, flags: unsafe { *flags.add(i) }, id: unsafe { *ids.add(i) } });
    }
    let _ = tx.send(out);
}

pub(crate) fn start(engine: &Arc<Engine>, matcher: Arc<dyn WalkMatcher>, sink: Arc<dyn Fn(EngineEvent) + Send + Sync>, since: u64) -> WatchHandle {
    let (tx, rx) = channel::<Vec<Raw>>();
    let ctx = Box::into_raw(Box::new(tx));
    let root = engine.watch_root();
    unsafe {
        let p = cfstring(&root);
        let arr = CFArrayCreate(std::ptr::null(), &p as *const CFStringRef as *const *const c_void, 1, &kCFTypeArrayCallBacks);
        CFRelease(p);
        let context = FSEventStreamContext { version: 0, info: ctx as *mut c_void, retain: std::ptr::null(), release: release_ctx as *const c_void, copy_description: std::ptr::null() };
        let stream = FSEventStreamCreate(std::ptr::null(), callback, &context, arr, since, 1.0, 0);
        CFRelease(arr);
        if stream.is_null() {
            drop(Box::from_raw(ctx));
            return WatchHandle { stream: std::ptr::null_mut(), queue: std::ptr::null_mut(), stop: Arc::new(AtomicBool::new(true)), worker: None };
        }
        let queue = dispatch_queue_create(c"app.disko.fsevents".as_ptr(), std::ptr::null());
        FSEventStreamSetDispatchQueue(stream, queue);
        FSEventStreamStart(stream);
        let eng = engine.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let st = stop.clone();
        let worker = std::thread::Builder::new().name("disko-watch".into()).spawn(move || worker(eng, rx, st, matcher, sink)).ok();
        WatchHandle { stream, queue, stop, worker }
    }
}

fn worker(engine: Arc<Engine>, rx: Receiver<Vec<Raw>>, stop: Arc<AtomicBool>, matcher: Arc<dyn WalkMatcher>, sink: Arc<dyn Fn(EngineEvent) + Send + Sync>) {
    let mut last_volume = Instant::now() - Duration::from_secs(60);
    loop {
        let mut batch = match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(b) => b,
            Err(RecvTimeoutError::Timeout) if !stop.load(Ordering::Relaxed) => continue,
            _ => return,
        };
        // Coalesce: keep collecting until 300 ms of quiet, at most ~1 s.
        let deadline = Instant::now() + Duration::from_millis(1000);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            match rx.recv_timeout(left.min(Duration::from_millis(300))) {
                Ok(more) => batch.extend(more),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let changed = engine.apply_events(batch, matcher.as_ref(), sink.as_ref());
        if !changed.is_empty() {
            sink(EngineEvent::Changed(changed));
            if last_volume.elapsed() > Duration::from_secs(5) {
                last_volume = Instant::now();
                sink(EngineEvent::Volume(engine.volume()));
            }
        }
    }
}

/// Debug helper: raw FSEvents (path, flags) under `root` for `secs` seconds, starting at `since`
/// (or now). Used by the `fsevents_probe` example; not part of the app's flow.
#[doc(hidden)]
pub fn debug_raw_events(root: &str, since: Option<u64>, secs: f64, during: impl FnOnce()) -> Vec<(String, u32, u64)> {
    let (tx, rx) = channel::<Vec<Raw>>();
    let ctx = Box::into_raw(Box::new(tx));
    let mut out = Vec::new();
    unsafe {
        let p = cfstring(root);
        let arr = CFArrayCreate(std::ptr::null(), &p as *const CFStringRef as *const *const c_void, 1, &kCFTypeArrayCallBacks);
        CFRelease(p);
        let context = FSEventStreamContext { version: 0, info: ctx as *mut c_void, retain: std::ptr::null(), release: release_ctx as *const c_void, copy_description: std::ptr::null() };
        let stream = FSEventStreamCreate(std::ptr::null(), callback, &context, arr, since.unwrap_or(kFSEventStreamEventIdSinceNow), 0.2, 0);
        CFRelease(arr);
        if stream.is_null() {
            drop(Box::from_raw(ctx));
            return out;
        }
        let queue = dispatch_queue_create(c"app.disko.fsevents-probe".as_ptr(), std::ptr::null());
        FSEventStreamSetDispatchQueue(stream, queue);
        FSEventStreamStart(stream);
        during();
        let end = Instant::now() + Duration::from_secs_f64(secs);
        while let Some(left) = end.checked_duration_since(Instant::now()) {
            if let Ok(b) = rx.recv_timeout(left) {
                out.extend(b.into_iter().map(|r| (r.path, r.flags, r.id)));
            }
        }
        FSEventStreamStop(stream);
        FSEventStreamInvalidate(stream);
        FSEventStreamRelease(stream);
        dispatch_release(queue);
    }
    out
}

/// Debug helper: the current FSEvents id.
#[doc(hidden)]
pub fn debug_current_event_id() -> u64 {
    current_event_id()
}
