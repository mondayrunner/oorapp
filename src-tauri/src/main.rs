// No console window next to the app on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `Oorapp mcp`: the MCP server over stdio, for Claude and other agents,
    // from the same executable as the app so every install has it.
    if std::env::args().nth(1).as_deref() == Some("mcp") {
        std::process::exit(match oorapp_core::mcp::run() {
            code if code == std::process::ExitCode::SUCCESS => 0,
            _ => 1,
        });
    }
    oorapp_app::run()
}
