use std::process::ExitCode;

const RESERVED: &str = "plasmid new: reserved — the plasmid-sdk WIT world is not frozen yet \
(91 plan step 1 freezes the control protocol and manifest grammar; the SDK surface is a \
deferred design). See p1/crates/plasmid-sdk/src/lib.rs.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("new") => {
            eprintln!("{RESERVED}");
            ExitCode::from(2)
        }
        Some("--help" | "-h" | "help") => {
            println!("plasmid — plasmid attachment verbs (P1 freeze groundwork)");
            println!();
            println!(
                "  plasmid new <name>    scaffold a plasmid crate (reserved, not implemented)"
            );
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("plasmid: unknown verb `{other}`; try `plasmid --help`");
            ExitCode::from(2)
        }
        None => {
            eprintln!("plasmid: a verb is required; try `plasmid --help`");
            ExitCode::from(2)
        }
    }
}
