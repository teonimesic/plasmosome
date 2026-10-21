use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::process::ExitCode;

const NAME_LIMIT: usize = 64;

const VERSION: &str = "0.1.0";

const MISSING_SECTIONS: [&str; 2] = [
    "add description: one or two plain sentences saying what the plasmid is for; write \
     description = \"Describe what this plasmid is for.\"",
    "add impl: the component that implements it; write [impl] followed by \
     wasm = \"component.wasm\"; a capability section such as [network] also satisfies this",
];

const HELP: &str = "\
plasmid — the command line for plasmid authors

Usage:
  plasmid new <name>    write <name>.toml, the plasmid's declaration, in the current directory

plasmid new fills in the id, the version and an empty description, and grants nothing. It
prints the sections the declaration still needs, one per line, writes no component until the
plasmid interface is frozen, and exits 2. It writes nothing and exits 1 when <name> is not a
plasmid name (lower-case letters and digits in words joined by single hyphens, at most 64
bytes) or when <name>.toml already exists.
";

fn main() -> ExitCode {
    let mut arguments = std::env::args_os().skip(1);
    let Some(verb) = arguments.next() else {
        eprintln!("plasmid: a verb is required; try `plasmid --help`");
        return ExitCode::from(2);
    };
    match verb.to_str() {
        Some("new") => new(arguments.collect()),
        Some("--help" | "-h" | "help") => {
            print!("{HELP}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!(
                "plasmid: unknown verb `{}`; try `plasmid --help`",
                verb.to_string_lossy()
            );
            ExitCode::from(2)
        }
    }
}

fn new(arguments: Vec<OsString>) -> ExitCode {
    let [name] = arguments.as_slice() else {
        eprintln!(
            "plasmid new: takes exactly one name, such as `plasmid new my-plasmid`; \
             try `plasmid --help`"
        );
        return ExitCode::from(2);
    };
    let Some(name) = name.to_str().filter(|name| is_plasmid_name(name)) else {
        eprintln!(
            "plasmid new: {name:?} is not a plasmid name, which is lower-case letters and digits \
             in words joined by single hyphens, at most {NAME_LIMIT} bytes, such as my-plasmid; \
             nothing was written"
        );
        return ExitCode::from(1);
    };
    let path = format!("{name}.toml");
    let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            eprintln!(
                "plasmid new: {path} already exists and is left untouched; nothing was written"
            );
            return ExitCode::from(1);
        }
        Err(error) => {
            eprintln!("plasmid new: cannot create {path}: {error}; nothing was written");
            return ExitCode::from(1);
        }
    };
    let declaration = format!("id = \"{name}\"\nversion = \"{VERSION}\"\ndescription = \"\"\n");
    if let Err(error) = file.write_all(declaration.as_bytes()) {
        drop(file);
        match std::fs::remove_file(&path) {
            Ok(()) => eprintln!(
                "plasmid new: cannot finish writing {path}: {error}; it is removed and nothing \
                 was written"
            ),
            Err(removal) => eprintln!(
                "plasmid new: cannot finish writing {path}: {error}; it is left behind because \
                 it cannot be removed: {removal}"
            ),
        }
        return ExitCode::from(1);
    }
    for section in MISSING_SECTIONS {
        println!("{section}");
    }
    eprintln!(
        "plasmid new: wrote {path} with no component; the component is not written until the \
         plasmid interface is frozen"
    );
    ExitCode::from(2)
}

fn is_plasmid_name(name: &str) -> bool {
    name.len() <= NAME_LIMIT
        && name.split('-').all(|word| {
            !word.is_empty()
                && word
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}
