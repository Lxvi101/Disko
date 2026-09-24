#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--scan") {
        std::process::exit(disko_lib::scan_cli(args));
    }
    if args.iter().any(|a| a == "--reports") {
        std::process::exit(disko_lib::reports_cli(args));
    }
    if args.iter().any(|a| a == "--classify") {
        std::process::exit(disko_lib::classify_cli(args));
    }
    if args.iter().any(|a| a == "--remnants") {
        std::process::exit(disko_lib::remnants_cli(args));
    }
    disko_lib::run();
}
