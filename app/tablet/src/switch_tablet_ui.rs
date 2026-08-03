use std::env;
use std::io;
use std::process::{Command, ExitCode};

const NATIVE_SERVICE: &str = "xochitl.service";
const REMARQUE_SERVICE: &str = "remarque-tablet.service";

fn run_systemctl(arguments: &[&str]) -> io::Result<()> {
    let status = Command::new("systemctl").args(arguments).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "systemctl {} exited with {status}",
            arguments.join(" ")
        )))
    }
}

fn switch_to_native() -> io::Result<()> {
    run_systemctl(&["stop", REMARQUE_SERVICE])?;
    run_systemctl(&["start", NATIVE_SERVICE])
}

fn switch_to_remarque() -> io::Result<()> {
    run_systemctl(&["start", REMARQUE_SERVICE])
}

fn print_usage() {
    eprintln!("usage: switch-tablet-ui native|remarque");
}

fn main() -> ExitCode {
    let result = match env::args().nth(1).as_deref() {
        Some("native") => switch_to_native(),
        Some("remarque") => switch_to_remarque(),
        _ => {
            print_usage();
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("switch_failed={error}");
            ExitCode::FAILURE
        }
    }
}
