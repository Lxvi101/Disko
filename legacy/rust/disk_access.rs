use std::{fs, path::Path};

/// A best-effort access probe, not a query of macOS's private permission database.
/// Open and immediately close the file without reading its contents. Prefer the
/// user's database: Unix permissions can block the system database even with FDA.
pub fn has_full_disk_access(home: &Path) -> bool {
    fs::File::open(home.join("Library/Application Support/com.apple.TCC/TCC.db")).is_ok()
        || fs::File::open("/Library/Application Support/com.apple.TCC/TCC.db").is_ok()
}

/// Make a fresh protected-directory access attempt in the app process so macOS
/// can attribute it to Disko. There is no public API that grants or prompts for
/// Full Disk Access. Registration in Settings is controlled by macOS; users may
/// still need to add the app manually. Never enumerate or read personal data.
pub fn request_full_disk_access(home: &Path) -> bool {
    let _ = fs::read_dir(home.join("Library/Safari"));
    let _ = fs::read_dir(home.join("Library/Application Support/com.apple.TCC"));
    has_full_disk_access(home)
}

pub fn app_bundle(executable: &Path) -> Option<&Path> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app")
        .then_some(bundle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_targets_the_running_bundle_including_spaces() {
        let exe = Path::new("/Applications/Disko Preview.app/Contents/MacOS/disko");
        assert_eq!(app_bundle(exe), Some(Path::new("/Applications/Disko Preview.app")));
        assert_eq!(app_bundle(Path::new("/project/src-tauri/target/debug/disko")), None);
        assert_eq!(app_bundle(Path::new("/project.app/target/debug/disko")), None);
    }
}
