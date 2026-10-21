use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};

use crate::{Exit, note};

const VERSION: &str = "0.1.0";

const MISSING_SECTIONS: [&str; 2] = [
    "add description: one or two plain sentences saying what the plasmid is for; write \
     description = \"Describe what this plasmid is for.\"",
    "add impl: the component that implements it; write [impl] followed by \
     wasm = \"component.wasm\"; a capability section such as [network] also satisfies this",
];

pub(crate) fn run(name: &str, stdout: &mut dyn Write, stderr: &mut dyn Write) -> Exit {
    let path = format!("{name}.toml");
    let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            note(
                stderr,
                &format!(
                    "plasmid new: {path} already exists and is left untouched; nothing was \
                     written\n"
                ),
            );
            return Exit::Unavailable;
        }
        Err(error) => {
            note(
                stderr,
                &format!("plasmid new: cannot create {path}: {error}; nothing was written\n"),
            );
            return Exit::Unavailable;
        }
    };
    let declaration = format!("id = \"{name}\"\nversion = \"{VERSION}\"\ndescription = \"\"\n");
    if let Err(error) = file.write_all(declaration.as_bytes()) {
        drop(file);
        let outcome = match std::fs::remove_file(&path) {
            Ok(()) => "it is removed and nothing was written".to_string(),
            Err(removal) => format!(
                "it is left behind and may hold part of the declaration, because it cannot be \
                 removed: {removal}"
            ),
        };
        note(
            stderr,
            &format!("plasmid new: cannot finish writing {path}: {error}; {outcome}\n"),
        );
        return Exit::OutcomeUnknown;
    }
    let sections: String = MISSING_SECTIONS
        .iter()
        .map(|section| format!("{section}\n"))
        .collect();
    if let Err(error) = stdout
        .write_all(sections.as_bytes())
        .and_then(|()| stdout.flush())
    {
        note(
            stderr,
            &format!(
                "plasmid new: wrote {path}, but cannot print the sections it still needs: {error}\n"
            ),
        );
        return Exit::OutcomeUnknown;
    }
    note(
        stderr,
        &format!(
            "plasmid new: wrote {path} with no component; the component is not written until the \
             plasmid interface is frozen\n"
        ),
    );
    Exit::Refused
}
