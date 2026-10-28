#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(dead_code)]

mod composer;
mod config;
mod detector;
mod hook;
mod startup;
mod tray;
mod wallpaper;

use std::env;
use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};

fn attach_parent_console() {
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

fn print_help() {
    attach_parent_console();
    println!("wallpaper-shuffler v{}", env!("CARGO_PKG_VERSION"));
    println!("Lightweight event-driven Windows wallpaper shuffler\n");
    println!("Usage: wallpaper-shuffler [OPTIONS]\n");
    println!("Options:");
    println!("  --register-startup    Register application to start automatically on Windows boot");
    println!("  --unregister-startup  Remove application from Windows startup");
    println!("  --help, -h            Print this help message");
    println!("  --version, -v         Print version information");
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() > 1 {
        match args[1].as_str() {
            "--register-startup" => {
                attach_parent_console();
                if let Ok(exe_path) = env::current_exe() {
                    match startup::register_startup(exe_path.as_os_str()) {
                        Ok(()) => println!(
                            "Successfully registered wallpaper-shuffler in Windows startup."
                        ),
                        Err(e) => eprintln!("Error registering startup: {}", e),
                    }
                }
                return;
            }
            "--unregister-startup" => {
                attach_parent_console();
                match startup::unregister_startup() {
                    Ok(()) => println!(
                        "Successfully unregistered wallpaper-shuffler from Windows startup."
                    ),
                    Err(e) => eprintln!("Error unregistering startup: {}", e),
                }
                return;
            }
            "--help" | "-h" => {
                print_help();
                return;
            }
            "--version" | "-v" => {
                attach_parent_console();
                println!("wallpaper-shuffler v{}", env!("CARGO_PKG_VERSION"));
                return;
            }
            _ => {}
        }
    }

    let _cfg = config::AppConfig::default();
    println!(
        "Starting wallpaper-shuffler daemon (v{})...",
        env!("CARGO_PKG_VERSION")
    );
}
