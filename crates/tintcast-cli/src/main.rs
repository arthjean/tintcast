use std::process::ExitCode;

fn main() -> ExitCode {
    eprintln!("usage: tintcast <command>");
    eprintln!("Tintcast's commands are not available in this build yet.");
    ExitCode::from(2)
}
