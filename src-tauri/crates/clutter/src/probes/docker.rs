//! Docker / OrbStack: BuildKit cache and unused images, measured by Docker itself. Only looked at
//! when the engine is already running (asking would start a stopped VM).

use crate::finding::{make, FindingExt};
use crate::util::{run, s, tool, Size};
use crate::Clutter;
use disko_core::{Category, Finding, Safety};
use std::time::Duration;

/// "17.12GB (100%)" → bytes.
pub fn parse_size(t: &str) -> u64 {
    let t = t.split_whitespace().next().unwrap_or("").trim();
    let idx = t.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(t.len());
    let (num, unit) = t.split_at(idx);
    let n: f64 = num.parse().unwrap_or(0.0);
    let mult = match unit.to_ascii_lowercase().as_str() {
        "kb" | "k" => 1e3,
        "mb" | "m" => 1e6,
        "gb" | "g" => 1e9,
        "tb" | "t" => 1e12,
        _ => 1.0,
    };
    (n * mult) as u64
}

pub fn probe(c: &Clutter) -> Vec<Finding> {
    if !c.env.tools {
        return vec![];
    }
    let procs = c.procs();
    let up = procs.list.iter().any(|p| p.exe.contains("OrbStack") || p.name == "com.docker.backend" || p.name == "dockerd" || p.exe.contains("colima") || p.name == "com.docker.virtualization");
    if !up {
        return vec![];
    }
    let Some(docker) = tool(&c.env.home, "docker") else { return vec![] };
    let Some(out) = run(&docker, &["system", "df", "--format", "{{json .}}"], &c.env.home, Duration::from_secs(20)) else { return vec![] };
    let mut found = vec![];
    for line in out.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let reclaim = parse_size(v["Reclaimable"].as_str().unwrap_or("0"));
        if reclaim < 50 * 1024 * 1024 {
            continue;
        }
        let (rule, title, why, args, safety): (&str, &str, &str, &[&str], Safety) = match v["Type"].as_str().unwrap_or("") {
            "Build Cache" => ("containers.build-cache", "Docker build cache", "Layers BuildKit kept from earlier image builds; the next build redoes changed steps.", &["builder", "prune", "-af"], Safety::Safe),
            "Images" => ("containers.images", "Unused Docker images", "Images no container uses. Pulled images come back with docker pull; ones you built and never pushed don't.", &["image", "prune", "-af"], Safety::Review),
            _ => continue, // containers and volumes hold state: never
        };
        let mut argv = vec![s(&docker)];
        argv.extend(args.iter().map(|a| a.to_string()));
        let argv: Vec<&str> = argv.iter().map(|a| a.as_str()).collect();
        found.push(
            make(rule, Category::Containers, title, "docker", vec![], Size { bytes: reclaim, files: 0, newest: None }, safety, safety == Safety::Safe, why)
                .keyed(rule)
                .detail(format!("docker {}", args.join(" ")))
                .command(&argv, &format!("docker {}", args.join(" "))),
        );
    }
    found
}

#[cfg(test)]
mod tests {
    #[test]
    fn sizes() {
        assert_eq!(super::parse_size("17.12GB (100%)"), 17_120_000_000);
        assert_eq!(super::parse_size("0B"), 0);
        assert_eq!(super::parse_size("512kB"), 512_000);
    }
}
