// SPDX-License-Identifier: Apache-2.0
//! `ghi`: headless CLI for the eval harness and tests. Commands arrive with phase 2/3.

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("--version") | Some("-V") | None => println!("ghi {}", ghi_cli::version()),
        Some(other) => {
            eprintln!("ghi: unknown argument `{other}`");
            std::process::exit(2);
        }
    }
}
