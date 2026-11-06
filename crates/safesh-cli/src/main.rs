//! safesh command line entry point.

use std::process::ExitCode;

fn main() -> ExitCode {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("safesh {}", env!("CARGO_PKG_VERSION"));
        ExitCode::SUCCESS
    } else {
        eprintln!("usage: safesh --version");
        ExitCode::from(2)
    }
}
