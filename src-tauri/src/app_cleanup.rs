//! Deliberately narrow, live app storage recipes. User state is measured, never offered.
use crate::xcode::{allocated_size, XcodeItem};
use serde::Serialize;
use std::{fs, path::{Path, PathBuf}, process::Command};

#[derive(Serialize)]
pub struct Inventory { pub total: u64, pub potential: u64, pub partial: bool, pub items: Vec<XcodeItem>, pub warnings: Vec<String> }
fn children(path: &Path) -> Vec<PathBuf> {
    fs::read_dir(path).into_iter().flatten().filter_map(Result::ok).map(|e| e.path()).collect()
}
fn name(path: &Path) -> String { path.file_name().unwrap_or_default().to_string_lossy().into() }
fn version_key(value: &str) -> Option<Vec<u64>> {
    let numeric = value.split('-').next()?;
    let parts: Vec<_> = numeric.split('.').map(str::parse::<u64>).collect::<Result<_,_>>().ok()?;
    (parts.len() == 3).then_some(parts)
}
fn agent_paths(home: &Path, id: &str) -> (PathBuf, PathBuf) {
    if id == "cursor" { (home.join(".local/share/cursor-agent/versions"), home.join(".local/bin/cursor-agent")) }
    else { (home.join(".local/share/claude/versions"), home.join(".local/bin/claude")) }
}
fn processes() -> Result<String, String> {
    let out = Command::new("/bin/ps").args(["-axo", "comm=", "-ww"]).output().map_err(|e| e.to_string())?;
    if !out.status.success() { return Err("Cannot verify running apps. Refresh when process access is available.".into()); }
    Ok(String::from_utf8_lossy(&out.stdout).to_lowercase())
}
fn running(id: &str, list: &str) -> bool {
    let needles: &[&str] = match id {
        "helium" => &["helium.app/", "/helium helper"],
        "adobe" => &["adobe after effects", "adobe premiere", "adobe media encoder", "aerender", "pproheadless"],
        "cursor" => &["cursor.app/", "cursor-agent"],
        _ => &["claude.app/", "/claude/versions/", "/bin/claude"],
    };
    list.lines().any(|line| needles.iter().any(|n| line.contains(n)) || (id == "claude" && line.trim() == "claude"))
}
fn version_block(path: &Path, root: &Path, launcher: &Path, process_list: &Result<String,String>, id: &str) -> Option<String> {
    if path.parent() != Some(root) || version_key(&name(path)).is_none() { return Some("Unrecognized installation layout; keep this entry.".into()); }
    let current = match fs::canonicalize(launcher) { Ok(p) if p.starts_with(root) => p, _ => return Some("Current launcher could not be verified; keep installed versions.".into()) };
    if current.starts_with(path) { return Some("Current installation — kept.".into()); }
    let newest = children(root).iter().filter_map(|p| version_key(&name(p))).max();
    if version_key(&name(path)) == newest { return Some("Newest installed version — kept for updates.".into()); }
    match process_list {
        Err(e) => Some(e.clone()),
        Ok(list) if running(id,list) => Some("Quit this app and its agent sessions, then refresh.".into()),
        _ => None,
    }
}
fn cache_paths(home: &Path, id: &str) -> Vec<(PathBuf,String)> {
    let mut result = vec![];
    if id == "helium" {
        for profile in children(&home.join("Library/Caches/net.imput.helium")) {
            let label = name(&profile);
            if label == "Default" || label == "System Profile" || label.starts_with("Profile ") {
                for cache in ["Cache", "Code Cache", "GPUCache"] { result.push((profile.join(cache),format!("{label} · {cache}"))); }
            }
        }
    } else if id == "adobe" {
        for cache in ["Media Cache Files", "Media Cache", "Peak Files", "Analyzer Cache Files"] {
            result.push((home.join("Library/Application Support/Adobe/Common").join(cache),cache.into()));
        }
        for version in children(&home.join("Library/Caches/Adobe/After Effects")) {
            for cache in children(&version) {
                if name(&cache).starts_with("Disk Cache") { result.push((cache,format!("After Effects {} · Disk cache",name(&version)))); }
            }
        }
    } else {
        let app = if id == "cursor" { "Cursor" } else { "Claude" };
        for cache in ["Cache", "Code Cache", "GPUCache"] {
            result.push((home.join("Library/Application Support").join(app).join(cache),format!("Desktop · {cache}")));
        }
    }
    result
}
/// Re-run protection during both quarantine and permanent deletion, including Files actions.
pub fn validate(path: &Path, home: &Path) -> Result<(),String> {
    for id in ["cursor", "claude"] {
        let (root, launcher) = agent_paths(home,id);
        if path.starts_with(&root) || root.starts_with(path) {
            if let Some(reason) = version_block(path,&root,&launcher,&processes(),id) { return Err(reason); }
        }
    }
    for id in ["helium", "adobe", "cursor", "claude"] {
        if cache_paths(home,id).iter().any(|(cache,_)| path.starts_with(cache) || cache.starts_with(path)) {
            if running(id,&processes()?) { return Err("Quit the owning app and its agent sessions before cleaning its caches.".into()); }
        }
    }
    Ok(())
}
pub fn inventory(home: &Path, id: &str) -> Result<Inventory,String> {
    let mut roots: Vec<PathBuf> = match id {
        "helium" => vec![PathBuf::from("/Applications/Helium.app"), home.join("Library/Application Support/net.imput.helium"), home.join("Library/Caches/net.imput.helium")],
        "adobe" => {
            let mut paths: Vec<_> = children(Path::new("/Applications")).into_iter().filter(|p| name(p).starts_with("Adobe ")).collect();
            paths.extend([PathBuf::from("/Library/Application Support/Adobe"),home.join("Library/Application Support/Adobe"),home.join("Library/Caches/Adobe")]); paths
        },
        "cursor" => vec![PathBuf::from("/Applications/Cursor.app"),home.join(".cursor"),home.join(".local/share/cursor-agent"),home.join("Library/Application Support/Cursor")],
        "claude" => vec![PathBuf::from("/Applications/Claude.app"),home.join(".claude"),home.join(".local/share/claude"),home.join("Library/Application Support/Claude")],
        _ => return Err("Unknown app storage profile".into()),
    };
    // All footprint roots are disjoint; symlink aliases are never traversed.
    roots.sort(); roots.dedup();
    let mut report = Inventory { total:0,potential:0,partial:false,items:vec![],warnings:vec![] };
    for root in roots {
        match fs::symlink_metadata(&root) {
            Ok(meta) if !meta.file_type().is_symlink() => { let (size,partial)=allocated_size(&root); report.total+=size; report.partial |= partial; },
            Ok(_) => { report.partial=true; },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(_) => report.partial=true,
        }
    }
    let process_list = processes();
    for (path,label) in cache_paths(home,id) {
        if !path.exists() { continue; }
        let blocked = match &process_list { Err(e)=>Some(e.clone()), Ok(list) if running(id,list)=>Some("Quit the app, then refresh to clean this cache.".into()), _=>None };
        add_item(&mut report,path,label,"cache",blocked,true);
    }
    if id == "cursor" || id == "claude" {
        let (root,launcher)=agent_paths(home,id);
        for path in children(&root) {
            let eligible=version_block(&path,&root,&launcher,&Ok(String::new()),id).is_none();
            let blocked=version_block(&path,&root,&launcher,&process_list,id);
            let label=format!("Agent {}",name(&path));
            add_item(&mut report,path,label,"version",blocked,eligible);
        }
    }
    report.items.sort_by(|a,b| b.total.cmp(&a.total));
    if report.partial { report.warnings.push("Some locations could not be fully measured. Storage shown is a lower bound.".into()); }
    Ok(report)
}
fn add_item(report: &mut Inventory,path:PathBuf,label:String,group:&str,mut blocked:Option<String>, eligible:bool) {
    let (total,partial)=allocated_size(&path);
    if fs::canonicalize(&path).ok().as_deref() != Some(path.as_path()) { blocked=Some("Symbolic link or inaccessible location — kept.".into()); }
    if partial { blocked=Some("Incomplete access — inspect permissions before cleanup.".into()); }
    if eligible && !partial && fs::canonicalize(&path).ok().as_deref() == Some(path.as_path()) { report.potential += total; }
    report.items.push(XcodeItem { path:path.to_string_lossy().into(),name:label,group:group.into(),total,modified:None,last_booted:None,subtitle:if group=="version" { "Older agent installation" } else { "Generated cache · rebuilt when needed" }.into(),badges:vec![],blocked,partial });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    #[test]
    fn current_newest_unknown_and_running_versions_are_kept() {
        let base=fs::canonicalize(std::env::temp_dir()).unwrap();
        let home=base.join(format!("disko-apps-{}",std::process::id()));
        let (root,launcher)=agent_paths(&home,"claude");
        fs::create_dir_all(&root).unwrap(); fs::create_dir_all(launcher.parent().unwrap()).unwrap();
        for v in ["2.1.9","2.1.10","2.1.11","unknown"] { fs::write(root.join(v),b"binary").unwrap(); }
        symlink(root.join("2.1.10"),&launcher).unwrap();
        let idle=Ok(String::new());
        assert!(validate(&root.join("2.1.10"), &home).is_err());
        assert!(validate(&root, &home).is_err());
        assert!(version_block(&root.join("2.1.9"),&root,&launcher,&idle,"claude").is_none());
        for v in ["2.1.10","2.1.11","unknown","2.1.9/internal"] { assert!(version_block(&root.join(v),&root,&launcher,&idle,"claude").is_some()); }
        assert!(version_block(&root.join("2.1.9"),&root,&launcher,&Ok("claude".into()),"claude").is_some());
        assert!(version_block(&root.join("2.1.9"),&root,&launcher,&Err("no access".into()),"claude").is_some());
        fs::remove_file(&launcher).unwrap();
        assert!(version_block(&root.join("2.1.9"),&root,&launcher,&idle,"claude").is_some());
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    #[ignore = "Read-only local audit; explicitly opt in with DISKO_AUDIT_HOME"]
    fn live_inventory_audit() {
        let home = std::env::var("DISKO_AUDIT_HOME").expect("Explicit audit home required");
        for id in ["adobe", "helium", "cursor", "claude"] {
            let report = inventory(Path::new(&home), id).unwrap();
            println!("{id}: {}", serde_json::to_string(&report).unwrap());
            assert!(report.potential <= report.total);
        }
    }
    #[test]
    fn cache_recipes_exclude_profiles_updaters_and_presets() {
        let home=std::env::temp_dir().join(format!("disko-recipes-{}",std::process::id()));
        for p in ["Library/Caches/net.imput.helium/Default/Cache","Library/Caches/net.imput.helium/org.sparkle-project.Sparkle/Installation","Library/Caches/Adobe/After Effects/26.3/Disk Cache-test","Library/Caches/Adobe/After Effects/26.3/Presets"] { fs::create_dir_all(home.join(p)).unwrap(); }
        let helium=cache_paths(&home,"helium"); assert!(helium.iter().any(|(p,_)|p.ends_with("Default/Cache"))); assert!(!helium.iter().any(|(p,_)|p.to_string_lossy().contains("Sparkle")));
        let adobe=cache_paths(&home,"adobe"); assert!(adobe.iter().any(|(p,_)|p.ends_with("Disk Cache-test"))); assert!(!adobe.iter().any(|(p,_)|p.ends_with("Presets")));
        fs::remove_dir_all(home).unwrap();
    }
}
