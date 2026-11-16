//! The few CoreFoundation / CoreServices / libdispatch calls the engine needs.
#![allow(non_upper_case_globals, non_snake_case, dead_code)]

use std::ffi::{c_char, c_void, CStr, CString};

pub type CFTypeRef = *const c_void;
pub type CFStringRef = *const c_void;
pub type CFArrayRef = *const c_void;
pub type CFURLRef = *const c_void;
pub type CFAllocatorRef = *const c_void;
pub type CFIndex = isize;
pub type Boolean = u8;
pub type FSEventStreamRef = *mut c_void;
pub type FSEventStreamEventId = u64;
pub type DispatchQueue = *mut c_void;

pub const kFSEventStreamEventIdSinceNow: u64 = 0xFFFF_FFFF_FFFF_FFFF;
pub const kFSEventStreamCreateFlagNoDefer: u32 = 0x02;
pub const kFSEventStreamCreateFlagWatchRoot: u32 = 0x04;

pub const kFSEventStreamEventFlagMustScanSubDirs: u32 = 0x01;
pub const kFSEventStreamEventFlagUserDropped: u32 = 0x02;
pub const kFSEventStreamEventFlagKernelDropped: u32 = 0x04;
pub const kFSEventStreamEventFlagEventIdsWrapped: u32 = 0x08;
pub const kFSEventStreamEventFlagHistoryDone: u32 = 0x10;
pub const kFSEventStreamEventFlagRootChanged: u32 = 0x20;
pub const kFSEventStreamEventFlagMount: u32 = 0x40;
pub const kFSEventStreamEventFlagUnmount: u32 = 0x80;

const kCFStringEncodingUTF8: u32 = 0x0800_0100;
const kCFNumberSInt64Type: CFIndex = 4;

#[repr(C)]
pub struct FSEventStreamContext {
    pub version: CFIndex,
    pub info: *mut c_void,
    pub retain: *const c_void,
    pub release: *const c_void,
    pub copy_description: *const c_void,
}

pub type FSEventStreamCallback = extern "C" fn(FSEventStreamRef, *mut c_void, usize, *mut c_void, *const u32, *const u64);

#[repr(C)]
pub struct CFArrayCallBacks {
    _private: [u8; 0],
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    pub static kCFTypeArrayCallBacks: CFArrayCallBacks;
    pub static kCFURLVolumeAvailableCapacityForImportantUsageKey: CFStringRef;
    pub fn CFRelease(cf: CFTypeRef);
    pub fn CFArrayCreate(alloc: CFAllocatorRef, values: *const *const c_void, n: CFIndex, cb: *const CFArrayCallBacks) -> CFArrayRef;
    pub fn CFStringCreateWithCString(alloc: CFAllocatorRef, s: *const c_char, enc: u32) -> CFStringRef;
    pub fn CFStringGetCString(s: CFStringRef, buf: *mut c_char, size: CFIndex, enc: u32) -> Boolean;
    pub fn CFUUIDCreateString(alloc: CFAllocatorRef, uuid: CFTypeRef) -> CFStringRef;
    pub fn CFURLCreateFromFileSystemRepresentation(alloc: CFAllocatorRef, buf: *const u8, len: CFIndex, is_dir: Boolean) -> CFURLRef;
    pub fn CFURLCopyResourcePropertyForKey(url: CFURLRef, key: CFStringRef, value: *mut CFTypeRef, err: *mut CFTypeRef) -> Boolean;
    pub fn CFNumberGetValue(n: CFTypeRef, kind: CFIndex, out: *mut c_void) -> Boolean;
}

#[link(name = "CoreServices", kind = "framework")]
extern "C" {
    pub fn FSEventStreamCreate(alloc: CFAllocatorRef, cb: FSEventStreamCallback, ctx: *const FSEventStreamContext, paths: CFArrayRef, since: FSEventStreamEventId, latency: f64, flags: u32) -> FSEventStreamRef;
    pub fn FSEventStreamSetDispatchQueue(s: FSEventStreamRef, q: DispatchQueue);
    pub fn FSEventStreamStart(s: FSEventStreamRef) -> Boolean;
    pub fn FSEventStreamFlushSync(s: FSEventStreamRef);
    pub fn FSEventStreamStop(s: FSEventStreamRef);
    pub fn FSEventStreamInvalidate(s: FSEventStreamRef);
    pub fn FSEventStreamRelease(s: FSEventStreamRef);
    pub fn FSEventsGetCurrentEventId() -> FSEventStreamEventId;
    pub fn FSEventsCopyUUIDForDevice(dev: libc::dev_t) -> CFTypeRef;
}

extern "C" {
    pub fn dispatch_queue_create(label: *const c_char, attr: *const c_void) -> DispatchQueue;
    pub fn dispatch_release(obj: *mut c_void);
}

pub fn cfstring(s: &str) -> CFStringRef {
    let c = CString::new(s).unwrap_or_default();
    unsafe { CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), kCFStringEncodingUTF8) }
}

pub fn string_of(s: CFStringRef) -> Option<String> {
    if s.is_null() {
        return None;
    }
    let mut buf = vec![0 as c_char; 256];
    let ok = unsafe { CFStringGetCString(s, buf.as_mut_ptr(), buf.len() as CFIndex, kCFStringEncodingUTF8) };
    (ok != 0).then(|| unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned())
}

/// The system-wide latest FSEvents id.
pub fn current_event_id() -> u64 {
    unsafe { FSEventsGetCurrentEventId() }
}

/// FSEvents' UUID for a device's event history; changes when the history is reset.
pub fn device_uuid(dev: u64) -> String {
    unsafe {
        let u = FSEventsCopyUUIDForDevice(dev as libc::dev_t);
        if u.is_null() {
            return String::new();
        }
        let s = CFUUIDCreateString(std::ptr::null(), u);
        CFRelease(u);
        let out = string_of(s).unwrap_or_default();
        if !s.is_null() {
            CFRelease(s);
        }
        out
    }
}

/// Free bytes "for important usage" (free + what macOS can purge on demand).
pub fn important_available(path: &str) -> Option<u64> {
    unsafe {
        let url = CFURLCreateFromFileSystemRepresentation(std::ptr::null(), path.as_ptr(), path.len() as CFIndex, 1);
        if url.is_null() {
            return None;
        }
        let mut v: CFTypeRef = std::ptr::null();
        let ok = CFURLCopyResourcePropertyForKey(url, kCFURLVolumeAvailableCapacityForImportantUsageKey, &mut v, std::ptr::null_mut());
        CFRelease(url);
        if ok == 0 || v.is_null() {
            return None;
        }
        let mut n: i64 = 0;
        let got = CFNumberGetValue(v, kCFNumberSInt64Type, &mut n as *mut i64 as *mut c_void);
        CFRelease(v);
        (got != 0 && n >= 0).then_some(n as u64)
    }
}
