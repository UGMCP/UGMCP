#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() == Some("serve") {
        let rest: Vec<String> = args.collect();
        std::process::exit(unit_agent::serve(&rest));
    }
    unit_agent::run();
}
