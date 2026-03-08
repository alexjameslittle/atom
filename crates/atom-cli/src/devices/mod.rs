pub(crate) mod android;
pub(crate) mod ios;

use std::io::{self, IsTerminal, Write};

use atom_ffi::{AtomError, AtomErrorCode, AtomResult};

pub(crate) fn choose_from_menu<T, F>(title: &str, options: &[T], render: F) -> AtomResult<T>
where
    T: Clone,
    F: Fn(&T) -> String,
{
    if options.is_empty() {
        return Err(AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            format!("{title} could not find any choices"),
        ));
    }

    if options.len() == 1 {
        return Ok(options[0].clone());
    }

    let mut stdout = io::stdout();
    writeln!(stdout, "{title}:").map_err(|error| io_error_to_cli_error(&error))?;
    for (index, option) in options.iter().enumerate() {
        writeln!(stdout, "  {}. {}", index + 1, render(option))
            .map_err(|error| io_error_to_cli_error(&error))?;
    }
    loop {
        write!(stdout, "Enter selection [1-{}]: ", options.len())
            .map_err(|error| io_error_to_cli_error(&error))?;
        stdout
            .flush()
            .map_err(|error| io_error_to_cli_error(&error))?;

        let mut line = String::new();
        io::stdin()
            .read_line(&mut line)
            .map_err(|error| io_error_to_cli_error(&error))?;
        let trimmed = line.trim();
        if let Ok(selection) = trimmed.parse::<usize>()
            && (1..=options.len()).contains(&selection)
        {
            return Ok(options[selection - 1].clone());
        }
        writeln!(stdout, "Invalid selection.").map_err(|error| io_error_to_cli_error(&error))?;
    }
}

pub(crate) fn should_prompt_interactively() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

fn io_error_to_cli_error(error: &io::Error) -> AtomError {
    AtomError::new(
        AtomErrorCode::ExternalToolFailed,
        format!("interactive device selection failed: {error}"),
    )
}
