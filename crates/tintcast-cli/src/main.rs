use std::path::PathBuf;
use std::process::ExitCode;

use tintcast_windows::engine::{DataDir, ImportOutcome, ImportReport, Library, LoadReport};

const USAGE: &str = "usage: tintcast <command>

Commands:
  list                        Lists the themes
  import <path> [--replace]   Imports themes";

enum Command {
    List,
    Import { path: PathBuf, replace: bool },
}

fn parse(args: &[String]) -> Option<Command> {
    match args {
        [command] if command == "list" => Some(Command::List),
        [command, rest @ ..] if command == "import" => {
            let mut path = None;
            let mut replace = false;
            for arg in rest {
                match arg.as_str() {
                    "--replace" => replace = true,
                    flag if flag.starts_with("--") => return None,
                    value if path.is_none() => path = Some(PathBuf::from(value)),
                    _ => return None,
                }
            }
            path.map(|path| Command::Import { path, replace })
        }
        _ => None,
    }
}

fn open() -> Result<Library, String> {
    let data = DataDir::open().map_err(|e| e.to_string())?;
    Library::open(data).map_err(|e| e.to_string())
}

fn print_load_report(report: &LoadReport) {
    for problem in &report.problems {
        eprintln!("warning: {problem}");
    }
    for rejected in &report.rejected {
        eprintln!("warning: {rejected}");
    }
    if let Some(reason) = &report.prune_skipped {
        eprintln!("warning: stored wallpapers were not cleaned up: {reason}");
    }
}

fn list(library: &Library) {
    for theme in library.themes() {
        let origin = if theme.is_bundled() { ", bundled" } else { "" };
        let count = theme.wallpapers.len();
        let noun = if count == 1 {
            "wallpaper"
        } else {
            "wallpapers"
        };
        println!(
            "{}  ({}, {}, {count} {noun}{origin})",
            theme.name,
            theme.id,
            theme.mode().label()
        );
    }
}

fn print_import(report: &ImportReport) {
    for entry in &report.skipped_entries {
        println!("{entry}");
    }
    for item in &report.items {
        match &item.outcome {
            ImportOutcome::Imported {
                wallpapers,
                replaced,
            } => {
                let verb = if *replaced { "Replaced" } else { "Imported" };
                let slug = item.slug.as_deref().unwrap_or_default();
                let noun = if *wallpapers == 1 {
                    "wallpaper"
                } else {
                    "wallpapers"
                };
                println!("{verb} {} as {slug} ({wallpapers} {noun})", item.name);
            }
            ImportOutcome::Skipped(reason) => println!("Skipped {}: {reason}", item.name),
        }
        for rejected in &item.rejected {
            println!("  {rejected}");
        }
        for entry in &item.skipped_entries {
            println!("  {entry}");
        }
        for warning in &item.warnings {
            println!("  {warning}");
        }
    }
    println!(
        "{} of {} themes imported.",
        report.imported(),
        report.items.len()
    );
}

fn run(command: Command) -> Result<(), String> {
    let mut library = open()?;
    print_load_report(library.report());
    match command {
        Command::List => list(&library),
        Command::Import { path, replace } => {
            let report = library.import(&path, replace).map_err(|e| e.to_string())?;
            print_import(&report);
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = parse(&args) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    match run(command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn parses_list_and_import() {
        assert!(matches!(parse(&args(&["list"])), Some(Command::List)));
        assert!(matches!(
            parse(&args(&["import", "C:\\themes"])),
            Some(Command::Import { replace: false, .. })
        ));
        assert!(matches!(
            parse(&args(&["import", "--replace", "C:\\themes"])),
            Some(Command::Import { replace: true, .. })
        ));
    }

    #[test]
    fn rejects_bad_usage() {
        for bad in [
            &[][..],
            &["import"],
            &["import", "a", "b"],
            &["import", "a", "--force"],
            &["apply"],
        ] {
            assert!(parse(&args(bad)).is_none(), "{bad:?}");
        }
    }
}
