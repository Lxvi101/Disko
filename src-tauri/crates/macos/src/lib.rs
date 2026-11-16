//! macOS glue for the app shell: app/file icons as PNG data URLs, Finder, Quick Look, Full Disk Access.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Icon of any file or app bundle as `data:image/png;base64,…`, `size` points square (rendered @2x).
/// Cached on disk under the Disko cache dir, keyed by path + mtime + size.
pub fn icon_data_url(path: &str, size: u32) -> Option<String> {
    let p = Path::new(path);
    let meta = std::fs::metadata(p).ok()?;
    let mtime = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
    let key = disko_core::finding_id("icon", &[format!("{}|{}|{}", path, mtime, size)]);
    let dir = disko_core::paths::cache_dir().join("icons");
    let file = dir.join(format!("{}.png", key.trim_start_matches("icon:")));
    let png = match std::fs::read(&file) {
        Ok(b) if !b.is_empty() => b,
        _ => {
            let b = render_icon(path, size)?;
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(&file, &b);
            b
        }
    };
    Some(format!("data:image/png;base64,{}", base64(&png)))
}

fn render_icon(path: &str, size: u32) -> Option<Vec<u8>> {
    use objc2::AnyThread;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSWorkspace};
    use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString};
    let ws = NSWorkspace::sharedWorkspace();
    let img = ws.iconForFile(&NSString::from_str(path));
    let px = (size * 2) as f64;
    let mut rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(px, px));
    // SAFETY: rect is a valid pointer for the call; no context or hints.
    let cg = unsafe { img.CGImageForProposedRect_context_hints(&mut rect, None, None) }?;
    let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &cg);
    // SAFETY: empty properties dictionary of the right type.
    let data = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }?;
    Some(data.to_vec())
}

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

fn exists(path: &str) -> Result<(), String> {
    if std::fs::symlink_metadata(path).is_err() {
        return Err("It's gone already.".into());
    }
    Ok(())
}

pub fn reveal(path: &str) -> Result<(), String> {
    exists(path)?;
    Command::new("/usr/bin/open").arg("-R").arg(path).spawn().map(|_| ()).map_err(|e| e.to_string())
}

pub fn open(path: &str) -> Result<(), String> {
    exists(path)?;
    Command::new("/usr/bin/open").arg(path).spawn().map(|_| ()).map_err(|e| e.to_string())
}

pub fn quicklook(path: &str) -> Result<(), String> {
    exists(path)?;
    let mut child = Command::new("/usr/bin/qlmanage").arg("-p").arg(path).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|e| e.to_string())?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Best-effort probe: opening the TCC database only works with Full Disk Access. Reads nothing.
pub fn has_full_disk_access() -> bool {
    let home = disko_core::paths::home();
    std::fs::File::open(home.join("Library/Application Support/com.apple.TCC/TCC.db")).is_ok()
        || std::fs::File::open("/Library/Application Support/com.apple.TCC/TCC.db").is_ok()
}

/// Touch a protected folder so macOS lists Disko in the Full Disk Access pane, then open that pane.
pub fn open_full_disk_access() -> Result<(), String> {
    let home = disko_core::paths::home();
    let _ = std::fs::read_dir(home.join("Library/Safari"));
    let _ = std::fs::read_dir(home.join("Library/Application Support/com.apple.TCC"));
    Command::new("/usr/bin/open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles")
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// The running .app bundle, when Disko runs from one.
pub fn app_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    (macos.file_name()? == "MacOS" && contents.file_name()? == "Contents" && bundle.extension()? == "app").then(|| bundle.to_path_buf())
}

#[cfg(test)]
mod tests {
    #[test]
    fn renders_finder_icon() {
        let url = super::icon_data_url("/System/Library/CoreServices/Finder.app", 32).expect("icon");
        assert!(url.starts_with("data:image/png;base64,iVBOR"));
    }
    #[test]
    fn base64_matches() {
        assert_eq!(super::base64(b"Man"), "TWFu");
        assert_eq!(super::base64(b"Ma"), "TWE=");
        assert_eq!(super::base64(b"M"), "TQ==");
    }
}
