//! Run Haiku Setup's command line without the game window, for Linux
//! development and CI: check-game, build, make-module, ... (lib.rs `cli`).
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = haiku_setup::cli(&args).unwrap_or(0);
    std::process::exit(code);
}
