#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--mcp") {
        std::process::exit(disko_lib::mcp_cli());
    }
    if args.iter().any(|a| a == "--bench") {
        std::process::exit(disko_lib::bench_cli(&args));
    }
    if args.iter().any(|a| a == "--json") {
        std::process::exit(disko_lib::json_cli(&args));
    }
    disko_lib::run();
}
