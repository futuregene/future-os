// Hide the console window on Windows release builds — without this, launching
// the GUI also pops up a terminal showing the app/agent logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    futureos_lib::run()
}
