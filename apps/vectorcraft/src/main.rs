//! VectorCraft desktop app: the shared launcher (`src/lib.rs`) started as VectorCraft.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

fn main() -> std::process::ExitCode {
    vectorcraft_app::run(&vectorcraft_app::VECTORCRAFT)
}
