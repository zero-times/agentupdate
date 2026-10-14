#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if !agentupdate::deploy::helper_entry() {
        agentupdate::run();
    }
}
