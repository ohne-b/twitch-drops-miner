#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments.iter().any(|value| value == "--version") {
        println!(
            "Drops Miner {}{}",
            env!("CARGO_PKG_VERSION"),
            if cfg!(feature = "desktop-fixture") {
                " (offline fixture)"
            } else {
                ""
            }
        );
        return;
    }
    if arguments
        .iter()
        .any(|value| matches!(value.as_str(), "--offline-smoke" | "--offline-preview"))
        && !cfg!(feature = "desktop-fixture")
    {
        eprintln!("The offline fixture was not built. Refusing to start a real miner for testing.");
        std::process::exit(2);
    }
    twitch_drops_miner_desktop::run();
}
