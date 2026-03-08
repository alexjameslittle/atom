use std::io::{self, Write};

use atom_cli::run_process;

fn main() {
    let output = run_process();
    let _ = io::stdout().write_all(&output.stdout);
    let _ = io::stderr().write_all(&output.stderr);
    std::process::exit(output.exit_code);
}
