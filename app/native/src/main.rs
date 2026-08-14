#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    if let Some(code) = luna_mux_lib::try_run_hook_forwarder(&args) {
        std::process::exit(code);
    }
    if let Some(code) = luna_mux_lib::try_run_mcp_browser(&args) {
        std::process::exit(code);
    }
    luna_mux_lib::run();
}
