#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let mut args = std::env::args().skip(1);
    let first = args.next();
    match first.as_deref() {
        Some("serve") | Some("--headless") => {
            let rest: Vec<String> = args.collect();
            std::process::exit(unit_agent::serve(&rest));
        }
        Some("mcp") => {
            let rest: Vec<String> = args.collect();
            std::process::exit(unit_agent::mcp_main(&rest));
        }
        Some(
            command @ ("status" | "diagnostics" | "ai" | "workspace" | "exec" | "terminal" | "git"
            | "project"),
        ) => {
            let mut all = vec![command.to_string()];
            all.extend(args);
            std::process::exit(unit_agent::cli(&all));
        }
        Some("--help") | Some("-h") => {
            println!("{}", unit_agent::CLI_HELP);
        }
        _ => unit_agent::run(),
    }
}
