use clap::Parser;
use std::process::ExitCode;
use tasker::{Cli, OutputFormat, execute, render_command_success, render_error_line};

fn requested_format(args: &[String]) -> OutputFormat {
    for (i, arg) in args.iter().enumerate() {
        if let Some(value) = arg.strip_prefix("--output=") {
            return value.parse().unwrap_or(OutputFormat::Human);
        }
        if (arg == "--output" || arg == "-o") && args.get(i + 1).is_some() {
            return args[i + 1].parse().unwrap_or(OutputFormat::Human);
        }
    }
    std::env::var("TASKER_OUTPUT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(OutputFormat::Human)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let cli = match Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        Err(error) => {
            if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) {
                print!("{error}");
                return ExitCode::SUCCESS;
            }
            let format = requested_format(&args);
            let app_error = tasker::AppError::input(error.to_string());
            eprint!("{}", render_error_line(&app_error, format, false));
            return ExitCode::from(app_error.exit_code());
        }
    };
    let format = cli.effective_output();
    match execute(&cli) {
        Ok(value) => {
            print!("{}", render_command_success(&cli, &value));
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprint!("{}", render_error_line(&error, format, cli.pretty));
            ExitCode::from(error.exit_code())
        }
    }
}
