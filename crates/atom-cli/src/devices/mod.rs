pub(crate) mod android;
pub(crate) mod ios;

use std::io::{self, IsTerminal};

use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use dialoguer::Select;

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

    let labels: Vec<String> = options.iter().map(&render).collect();
    let selection = Select::new()
        .with_prompt(title)
        .items(&labels)
        .default(0)
        .interact()
        .map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("interactive device selection failed: {error}"),
            )
        })?;

    Ok(options[selection].clone())
}

pub(crate) fn should_prompt_interactively() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}
