use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

use serde_json::{Value, json};
use tintcast_windows::engine::{
    ApplyReport, ApplyRequest, DataDir, Engine, EngineLock, ImportOutcome, ImportReport,
    IntegrationResult, Library, LibraryOptions, LoadReport, Outcome, PlannedIntegration,
    PlannedOutcome, RestoreReport, RestoreResult, Startup, Theme, UndoReport,
};
use tintcast_windows::{System, describe};

pub const USAGE: &str = "usage: tintcast <command>

Commands:
  list                               Lists the themes
  plan <theme> [--wallpaper N]       Prints the preview and writes nothing
  apply <theme> [--wallpaper N]      Applies the theme
  undo                               Undoes the last apply
  restore                            Restores the original setup
  status [--json]                    Shows the current theme, the restore records, and any interrupted run
  import <path> [--replace]          Imports themes
  report [--json]                    Prints the system state Tintcast watches, without changing it";

pub const SUCCESS: u8 = 0;
pub const FAILED: u8 = 1;
pub const USAGE_ERROR: u8 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    List,
    Plan {
        theme: String,
        wallpaper: Option<usize>,
    },
    Apply {
        theme: String,
        wallpaper: Option<usize>,
    },
    Undo,
    Restore,
    Status {
        json: bool,
    },
    Import {
        path: PathBuf,
        replace: bool,
    },
    Report {
        json: bool,
    },
}

fn theme_args(rest: &[String]) -> Option<(String, Option<usize>)> {
    let mut theme = None;
    let mut wallpaper = None;
    let mut items = rest.iter();
    while let Some(arg) = items.next() {
        match arg.as_str() {
            "--wallpaper" if wallpaper.is_none() => {
                let n: usize = items.next()?.parse().ok()?;
                wallpaper = Some(n.checked_sub(1)?);
            }
            flag if flag.starts_with("--") => return None,
            value if theme.is_none() => theme = Some(value.to_owned()),
            _ => return None,
        }
    }
    theme.map(|theme| (theme, wallpaper))
}

fn json_flag(rest: &[String]) -> Option<bool> {
    match rest {
        [] => Some(false),
        [flag] if flag == "--json" => Some(true),
        _ => None,
    }
}

pub fn parse(args: &[String]) -> Option<Command> {
    let (command, rest) = args.split_first()?;
    match command.as_str() {
        "list" if rest.is_empty() => Some(Command::List),
        "plan" => theme_args(rest).map(|(theme, wallpaper)| Command::Plan { theme, wallpaper }),
        "apply" => theme_args(rest).map(|(theme, wallpaper)| Command::Apply { theme, wallpaper }),
        "undo" if rest.is_empty() => Some(Command::Undo),
        "restore" if rest.is_empty() => Some(Command::Restore),
        "status" => json_flag(rest).map(|json| Command::Status { json }),
        "report" => json_flag(rest).map(|json| Command::Report { json }),
        "import" => {
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

pub fn printable(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_control() {
            out.extend(c.escape_unicode());
        } else {
            out.push(c);
        }
    }
    out
}

pub struct Context {
    pub system: Box<dyn Fn() -> Result<System, String>>,
    pub data: Box<dyn Fn() -> Result<DataDir, String>>,
    pub library: LibraryOptions,
    pub lock: EngineLock,
}

impl Context {
    pub fn windows() -> Self {
        Self {
            system: Box::new(System::windows),
            data: Box::new(|| DataDir::open().map_err(|e| e.to_string())),
            library: LibraryOptions::default(),
            lock: EngineLock::default(),
        }
    }
}

struct Io<'a> {
    out: &'a mut dyn Write,
    err: &'a mut dyn Write,
}

impl Io<'_> {
    fn line(&mut self, text: impl AsRef<str>) {
        let _ = writeln!(self.out, "{}", printable(text.as_ref()));
    }

    fn json(&mut self, value: &impl serde::Serialize) -> Step<()> {
        let text = serde_json::to_string_pretty(value).map_err(failed)?;
        let _ = writeln!(self.out, "{text}");
        Ok(())
    }

    fn warn(&mut self, text: impl AsRef<str>) {
        let _ = writeln!(self.err, "{}", printable(text.as_ref()));
    }

    fn usage(&mut self) {
        let _ = writeln!(self.err, "{USAGE}");
    }
}

enum Failure {
    Usage(String),
    Failed(String),
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self::Failed(message)
    }
}

type Step<T> = Result<T, Failure>;

fn failed(error: impl std::fmt::Display) -> Failure {
    Failure::Failed(error.to_string())
}

struct Session<'c> {
    context: &'c Context,
    data: DataDir,
    system: System,
}

impl<'c> Session<'c> {
    fn open(context: &'c Context) -> Step<Self> {
        Ok(Self {
            data: (context.data)()?,
            system: (context.system)()?,
            context,
        })
    }

    fn library(&self, io: &mut Io) -> Step<Library> {
        let options = LibraryOptions {
            lock: self.context.lock.clone(),
            ..self.context.library.clone()
        };
        let library = Library::open_with(self.data.clone(), options).map_err(failed)?;
        print_load_report(io, library.report());
        Ok(library)
    }

    fn engine(&self, io: &mut Io) -> Step<Engine> {
        let engine = self
            .system
            .engine(self.data.clone())
            .with_lock(self.context.lock.clone());
        let startup = engine.startup().map_err(failed)?;
        warn_startup(io, &startup);
        Ok(engine)
    }
}

fn print_load_report(io: &mut Io, report: &LoadReport) {
    for problem in &report.problems {
        io.warn(format!("warning: {problem}"));
    }
    for rejected in &report.rejected {
        io.warn(format!("warning: {rejected}"));
    }
    if let Some(reason) = &report.prune_skipped {
        io.warn(format!(
            "warning: stored wallpapers were not cleaned up: {reason}"
        ));
    }
}

fn warn_startup(io: &mut Io, startup: &Startup) {
    for run in &startup.interrupted {
        io.warn(format!("warning: {} Run tintcast status.", run.message()));
    }
    for path in &startup.corrupt_journals {
        io.warn(format!(
            "warning: Tintcast set aside an unreadable journal as {}",
            path.display()
        ));
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        format!("{count} {one}")
    } else {
        format!("{count} {many}")
    }
}

fn list(io: &mut Io, library: &Library) {
    for theme in library.themes() {
        let origin = if theme.is_bundled() { ", bundled" } else { "" };
        let count = plural(theme.wallpapers.len(), "wallpaper", "wallpapers");
        io.line(format!(
            "{}  ({}, {}, {count}{origin})",
            theme.name,
            theme.id,
            theme.mode().label()
        ));
    }
}

fn print_import(io: &mut Io, report: &ImportReport) {
    for entry in &report.skipped_entries {
        io.line(entry);
    }
    for item in &report.items {
        match &item.outcome {
            ImportOutcome::Imported {
                wallpapers,
                replaced,
            } => {
                let verb = if *replaced { "Replaced" } else { "Imported" };
                let slug = item.slug.as_deref().unwrap_or_default();
                let count = plural(*wallpapers, "wallpaper", "wallpapers");
                io.line(format!("{verb} {} as {slug} ({count})", item.name));
            }
            ImportOutcome::Skipped(reason) => io.line(format!("Skipped {}: {reason}", item.name)),
        }
        for rejected in &item.rejected {
            io.line(format!("  {rejected}"));
        }
        for entry in &item.skipped_entries {
            io.line(format!("  {entry}"));
        }
        for warning in &item.warnings {
            io.line(format!("  {warning}"));
        }
    }
    io.line(format!(
        "{} of {} themes imported.",
        report.imported(),
        report.items.len()
    ));
}

fn request(library: &Library, name: &str, wallpaper: Option<usize>) -> Step<ApplyRequest> {
    let theme: Theme = library
        .find(name)
        .cloned()
        .ok_or_else(|| Failure::Usage(format!("No theme named {name}. Run tintcast list.")))?;
    let index = wallpaper.unwrap_or(0);
    let count = theme.wallpapers.len();
    if wallpaper.is_some() && index >= count {
        return Err(Failure::Usage(match count {
            0 => format!("{} has no wallpaper.", theme.name),
            1 => format!("{} has one wallpaper. Pick --wallpaper 1.", theme.name),
            _ => format!(
                "{} has {count} wallpapers. Pick --wallpaper 1 to {count}.",
                theme.name
            ),
        }));
    }
    Ok(ApplyRequest::new(theme).with_wallpaper(index))
}

fn wallpaper_label(request: &ApplyRequest) -> String {
    let count = request.theme.wallpapers.len();
    if count == 0 {
        String::new()
    } else {
        format!(" (wallpaper {} of {count})", request.wallpaper + 1)
    }
}

fn print_plan(io: &mut Io, request: &ApplyRequest, plan: &[PlannedIntegration]) {
    io.line(format!(
        "Plan for {}{}:",
        request.theme.name,
        wallpaper_label(request)
    ));
    let mut changes = 0;
    for item in plan {
        io.line("");
        match &item.outcome {
            PlannedOutcome::Changes { operations, notes } => {
                if operations.is_empty() {
                    io.line(format!("{}: nothing to change", item.name));
                } else {
                    io.line(&item.name);
                }
                for op in operations {
                    io.line(format!("  {}", op.summary()));
                }
                for note in notes {
                    io.line(format!("  Note: {note}"));
                }
                changes += operations.len();
            }
            PlannedOutcome::Skip(reason) => io.line(format!("{}: skipped. {reason}", item.name)),
            PlannedOutcome::Error(error) => {
                io.line(format!("{}: can't apply. {error}", item.name));
            }
        }
    }
    io.line("");
    io.line(format!(
        "{} planned. Nothing was changed.",
        plural(changes, "change", "changes")
    ));
}

fn print_result(io: &mut Io, result: &IntegrationResult) {
    let status = match &result.outcome {
        Outcome::Skipped(reason) => format!("Skipped. {reason}"),
        Outcome::Failed { .. } => format!("Failed. {}", result.outcome.message()),
        other => other.message(),
    };
    io.line(format!("{}: {status}", result.name));
    if result.outcome.changed() {
        for summary in &result.operations {
            io.line(format!("  {summary}"));
        }
    }
    for note in &result.notes {
        io.line(format!("  Note: {note}"));
    }
}

fn print_apply(io: &mut Io, report: &ApplyReport, started: Instant) -> u8 {
    let elapsed = started.elapsed().as_millis();
    if report.failed() {
        io.line(format!(
            "Applied {} in {elapsed} ms, with a failed part.",
            report.theme_name
        ));
    } else {
        io.line(format!("Applied {} in {elapsed} ms.", report.theme_name));
    }
    for result in &report.results {
        print_result(io, result);
    }
    if report.failed() { FAILED } else { SUCCESS }
}

fn print_restore(io: &mut Io, report: &RestoreReport) -> u8 {
    if report.items.is_empty() {
        io.line("Nothing to restore: Tintcast holds no changed setting.");
        return SUCCESS;
    }
    let count = |wanted: fn(&RestoreResult) -> bool| {
        report.items.iter().filter(|i| wanted(&i.result)).count()
    };
    io.line(format!(
        "Restored {}, kept {}, unchanged {}, failed {}.",
        plural(
            count(|r| *r == RestoreResult::Restored),
            "setting",
            "settings"
        ),
        count(|r| *r == RestoreResult::KeptYourChange),
        count(|r| *r == RestoreResult::Unchanged),
        count(|r| matches!(r, RestoreResult::Failed(_))),
    ));
    for item in &report.items {
        let label = describe(&item.resource);
        match &item.result {
            RestoreResult::Failed(_) => {
                io.line(format!("  {label}: {}", item.result.message()));
            }
            result => io.line(format!("  {}: {label}", result.message())),
        }
    }
    for (integration, reload) in &report.reloads {
        if let tintcast_windows::engine::ReloadResult::Failed(error) = reload {
            io.line(format!("  Note: {integration} did not reload: {error}"));
        }
    }
    if report.failed() { FAILED } else { SUCCESS }
}

fn status_json(engine: &Engine, startup: &Startup) -> Step<Value> {
    let history = engine.history().map_err(failed)?;
    let ledger = engine.ledger_entries().map(|l| l.len()).ok();
    let current = history.last().map(|entry| {
        json!({
            "theme_id": entry.theme_id,
            "theme_name": entry.theme_name,
            "wallpaper": entry.wallpaper + 1,
            "at": entry.at.to_string(),
        })
    });
    let interrupted: Vec<Value> = startup
        .interrupted
        .iter()
        .map(|run| {
            json!({
                "run_id": run.run_id,
                "theme_id": run.theme_id,
                "theme_name": run.theme_name,
                "started": run.started.to_string(),
                "applied": run.applied.iter().map(|s| s.summary.clone()).collect::<Vec<_>>(),
                "pending": run.pending.iter().map(|s| s.summary.clone()).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(json!({
        "version": 1,
        "current": current,
        "history_entries": history.len(),
        "ledger_entries": ledger,
        "ledger_error": startup.ledger_error,
        "interrupted": interrupted,
        "corrupt_journals": startup
            .corrupt_journals
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>(),
    }))
}

fn print_status(io: &mut Io, status: &Value) {
    match status["current"].as_object() {
        Some(current) => io.line(format!(
            "Current theme: {} (wallpaper {}), applied {}",
            current["theme_name"].as_str().unwrap_or_default(),
            current["wallpaper"],
            current["at"].as_str().unwrap_or_default()
        )),
        None => io.line("Current theme: none"),
    }
    match status["ledger_entries"].as_u64() {
        Some(count) => io.line(format!(
            "Restore records: {}",
            plural(
                usize::try_from(count).unwrap_or(usize::MAX),
                "setting",
                "settings"
            )
        )),
        None => io.line(format!(
            "Restore records: unreadable. {}",
            status["ledger_error"].as_str().unwrap_or_default()
        )),
    }
    let runs = status["interrupted"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if runs.is_empty() {
        io.line("Interrupted runs: none");
    }
    for run in runs {
        io.line(format!(
            "Interrupted run {}: Tintcast was interrupted while applying {}. {} applied, {} pending.",
            run["run_id"].as_str().unwrap_or_default(),
            run["theme_name"].as_str().unwrap_or_default(),
            run["applied"].as_array().map_or(0, Vec::len),
            run["pending"].as_array().map_or(0, Vec::len),
        ));
    }
}

fn print_report(io: &mut Io, report: &tintcast_windows::Report) {
    io.line(format!("Windows build: {}", report.build));
    for (key, value) in &report.watched {
        io.line(format!("{key} = {value}"));
    }
    for (key, value) in &report.info {
        io.line(format!("info {key} = {value}"));
    }
}

fn execute(command: Command, context: &Context, io: &mut Io) -> Step<u8> {
    match command {
        Command::Report { json } => {
            let report = (context.system)()?.report();
            if json {
                io.json(&report)?;
            } else {
                print_report(io, &report);
            }
            Ok(SUCCESS)
        }
        Command::List => {
            let session = Session::open(context)?;
            let library = session.library(io)?;
            list(io, &library);
            Ok(SUCCESS)
        }
        Command::Import { path, replace } => {
            let session = Session::open(context)?;
            let mut library = session.library(io)?;
            let report = library.import(&path, replace).map_err(failed)?;
            print_import(io, &report);
            Ok(SUCCESS)
        }
        Command::Plan { theme, wallpaper } => {
            let session = Session::open(context)?;
            let library = session.library(io)?;
            let request = request(&library, &theme, wallpaper)?;
            let engine = session.engine(io)?;
            print_plan(io, &request, &engine.plan(&request));
            Ok(SUCCESS)
        }
        Command::Apply { theme, wallpaper } => {
            let session = Session::open(context)?;
            let library = session.library(io)?;
            let request = request(&library, &theme, wallpaper)?;
            let engine = session.engine(io)?;
            let started = Instant::now();
            let report = engine.apply(&request).map_err(failed)?;
            Ok(print_apply(io, &report, started))
        }
        Command::Undo => {
            let session = Session::open(context)?;
            let library = session.library(io)?;
            let engine = session.engine(io)?;
            let started = Instant::now();
            match engine.undo(&|id| library.lookup(id)).map_err(failed)? {
                UndoReport::NothingToUndo => {
                    io.line(UndoReport::NOTHING_TO_UNDO);
                    Ok(SUCCESS)
                }
                UndoReport::Reapplied(report) => Ok(print_apply(io, &report, started)),
                UndoReport::Restored(report) => Ok(print_restore(io, &report)),
            }
        }
        Command::Restore => {
            let session = Session::open(context)?;
            let engine = session.engine(io)?;
            let report = engine.restore().map_err(failed)?;
            Ok(print_restore(io, &report))
        }
        Command::Status { json } => {
            let session = Session::open(context)?;
            let engine = session
                .system
                .engine(session.data.clone())
                .with_lock(context.lock.clone());
            let startup = engine.startup().map_err(failed)?;
            let status = status_json(&engine, &startup)?;
            if json {
                io.json(&status)?;
            } else {
                print_status(io, &status);
            }
            Ok(SUCCESS)
        }
    }
}

pub fn run(args: &[String], context: &Context, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let mut io = Io { out, err };
    let Some(command) = parse(args) else {
        io.usage();
        return USAGE_ERROR;
    };
    match execute(command, context, &mut io) {
        Ok(code) => code,
        Err(Failure::Usage(message)) => {
            io.warn(message);
            USAGE_ERROR
        }
        Err(Failure::Failed(message)) => {
            io.warn(message);
            FAILED
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
    fn parses_every_subcommand() {
        assert_eq!(parse(&args(&["list"])), Some(Command::List));
        assert_eq!(
            parse(&args(&["plan", "Tokyo Night"])),
            Some(Command::Plan {
                theme: "Tokyo Night".into(),
                wallpaper: None
            })
        );
        assert_eq!(
            parse(&args(&["apply", "ironwood", "--wallpaper", "2"])),
            Some(Command::Apply {
                theme: "ironwood".into(),
                wallpaper: Some(1)
            })
        );
        assert_eq!(
            parse(&args(&["apply", "--wallpaper", "3", "linen"])),
            Some(Command::Apply {
                theme: "linen".into(),
                wallpaper: Some(2)
            })
        );
        assert_eq!(parse(&args(&["undo"])), Some(Command::Undo));
        assert_eq!(parse(&args(&["restore"])), Some(Command::Restore));
        assert_eq!(
            parse(&args(&["status", "--json"])),
            Some(Command::Status { json: true })
        );
        assert_eq!(
            parse(&args(&["report"])),
            Some(Command::Report { json: false })
        );
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
            &["apply", "a", "b"],
            &["apply", "a", "--wallpaper"],
            &["apply", "a", "--wallpaper", "0"],
            &["apply", "a", "--wallpaper", "x"],
            &["apply", "a", "--wallpaper", "1", "--wallpaper", "2"],
            &["plan", "a", "--json"],
            &["undo", "now"],
            &["restore", "all"],
            &["status", "--yaml"],
            &["report", "x"],
            &["list", "all"],
            &["unknown"],
        ] {
            assert!(parse(&args(bad)).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn control_characters_are_escaped_before_printing() {
        assert_eq!(
            printable("Evil\u{1b}]52;c;AAAA\u{7} thème"),
            "Evil\\u{1b}]52;c;AAAA\\u{7} thème"
        );
        assert_eq!(printable("Tokyo Night"), "Tokyo Night");
    }
}
