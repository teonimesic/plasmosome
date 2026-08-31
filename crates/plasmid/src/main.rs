use std::process::ExitCode;

const RESERVED: &str = "plasmid new: reserved — the plasmid-sdk interface a scaffold would \
generate against is not frozen yet, so there is no shape to write. Follow \
https://github.com/teonimesic/plasmosome for when it is.";

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
