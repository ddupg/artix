use std::ffi::OsStr;
use std::ffi::OsString;
use std::io::IsTerminal;
use std::path::PathBuf;

use artix::config::{
    AppContext, UiMode, init_default_config_file, load_config, render_default_config_toml,
};
use artix::scan::scan_workspace_with_context;
use artix::ui::{build_overview_rows, run_tui_with_context};

#[derive(Debug)]
enum CliCommand {
    Run { roots: Vec<PathBuf> },
    InitConfig,
    PrintDefaultConfig,
    Help,
}

#[tokio::main]
async fn main() {
    let command = match parse_cli_command() {
        Ok(command) => command,
        Err(err) => {
            eprintln!("artix: {err}");
            std::process::exit(1);
        }
    };

    match command {
        CliCommand::InitConfig => {
            match init_default_config_file() {
                Ok(path) => {
                    println!("initialized config at {}", path.display());
                    return;
                }
                Err(err) => {
                    eprintln!("artix: {err}");
                    std::process::exit(1);
                }
            }
        }
        CliCommand::PrintDefaultConfig => {
            print!("{}", render_default_config_toml());
            return;
        }
        CliCommand::Help => {
            print!("{}", render_help_text());
            return;
        }
        CliCommand::Run { roots } => {
            run_app(roots).await;
        }
    }
}

async fn run_app(roots: Vec<PathBuf>) {
    let loaded = match load_config() {
        Ok(loaded) => loaded,
        Err(err) => {
            eprintln!("artix: {err}");
            std::process::exit(1);
        }
    };
    for warning in &loaded.warnings {
        eprintln!("artix: warning: {warning}");
    }
    let ctx = AppContext::new(loaded.config);
    let roots = if roots.is_empty() {
        vec![std::env::current_dir().expect("current working directory must be readable")]
    } else {
        roots
    };

    let should_run_tui = match ctx.config().ui.mode {
        UiMode::Plain => false,
        UiMode::Tui => true,
        UiMode::Auto => std::io::stdout().is_terminal(),
    };

    if should_run_tui {
        let start_dir = roots
            .first()
            .cloned()
            .expect("at least one root is always present");
        if let Err(err) = run_tui_with_context(start_dir, ctx.clone()).await {
            eprintln!("artix: {err}");
            std::process::exit(1);
        }
        // Ensure we exit promptly even if background blocking tasks are still running.
        std::process::exit(0);
    }

    let report = scan_workspace_with_context(&roots, &ctx).await;
    let rows = build_overview_rows(report.projects);

    for row in rows {
        println!(
            "{}\t{}\t{}",
            row.project_name, row.reclaimable_bytes, row.candidate_count
        );
    }
}

fn parse_cli_command() -> Result<CliCommand, String> {
    parse_cli_command_from(std::env::args_os().skip(1).collect())
}

fn render_help_text() -> String {
    concat!(
        "artix - developer workspace cleanup TUI\n\n",
        "USAGE:\n",
        "    artix [PATH ...]\n",
        "    artix init-config\n",
        "    artix --print-default-config\n",
        "    artix help\n",
        "    artix -h | --help\n\n",
        "COMMANDS:\n",
        "    help                    Show this help text\n",
        "    init-config             Write a default config file to ~/.config/artix/config.toml\n\n",
        "FLAGS:\n",
        "    -h, --help              Show this help text\n",
        "        --print-default-config\n",
        "                            Print the default TOML config to stdout\n\n",
        "BEHAVIOR:\n",
        "    With no command, artix scans the current directory.\n",
        "    In [ui].mode = \"auto\", it runs the TUI on interactive stdout and\n",
        "    falls back to plain tab-separated output otherwise.\n\n",
        "CONFIG PATHS:\n",
        "    Primary path: ~/.config/artix/config.toml\n",
        "    Fallback path: ~/.artix/config.toml\n",
    )
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::{CliCommand, parse_cli_command_from};
    use std::ffi::OsString;
    use std::path::PathBuf;

    #[test]
    fn parse_init_config_command() {
        let command = parse_cli_command_from(vec![OsString::from("init-config")]).unwrap();

        assert!(matches!(command, CliCommand::InitConfig));
    }

    #[test]
    fn parse_print_default_config_flag() {
        let command = parse_cli_command_from(vec![OsString::from("--print-default-config")]).unwrap();

        assert!(matches!(command, CliCommand::PrintDefaultConfig));
    }

    #[test]
    fn parse_help_command() {
        let command = parse_cli_command_from(vec![OsString::from("help")]).unwrap();

        assert!(matches!(command, CliCommand::Help));
    }

    #[test]
    fn parse_short_help_flag() {
        let command = parse_cli_command_from(vec![OsString::from("-h")]).unwrap();

        assert!(matches!(command, CliCommand::Help));
    }

    #[test]
    fn reject_extra_args_for_print_default_config() {
        let err = parse_cli_command_from(vec![
            OsString::from("--print-default-config"),
            OsString::from("/tmp/workspace"),
        ])
        .unwrap_err();

        assert_eq!(err, "--print-default-config does not accept additional arguments");
    }

    #[test]
    fn reject_extra_args_for_help_command() {
        let err = parse_cli_command_from(vec![
            OsString::from("help"),
            OsString::from("/tmp/workspace"),
        ])
        .unwrap_err();

        assert_eq!(err, "help does not accept additional arguments");
    }

    #[test]
    fn parse_paths_as_run_command() {
        let command = parse_cli_command_from(vec![
            OsString::from("/tmp/a"),
            OsString::from("/tmp/b"),
        ])
        .unwrap();

        match command {
            CliCommand::Run { roots } => {
                assert_eq!(roots, vec![PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")]);
            }
            _ => panic!("expected run command"),
        }
    }
}

fn parse_cli_command_from(args: Vec<OsString>) -> Result<CliCommand, String> {
    if args.is_empty() {
        return Ok(CliCommand::Run { roots: Vec::new() });
    }

    if args[0] == OsString::from("init-config") {
        if args.len() != 1 {
            return Err("init-config does not accept additional arguments".to_string());
        }
        return Ok(CliCommand::InitConfig);
    }

    if args[0] == OsString::from("help") {
        if args.len() != 1 {
            return Err("help does not accept additional arguments".to_string());
        }
        return Ok(CliCommand::Help);
    }

    if args
        .iter()
        .any(|arg| arg == OsStr::new("--print-default-config"))
    {
        if args.len() != 1 {
            return Err("--print-default-config does not accept additional arguments".to_string());
        }
        return Ok(CliCommand::PrintDefaultConfig);
    }

    if args
        .iter()
        .any(|arg| arg == OsStr::new("--help") || arg == OsStr::new("-h"))
    {
        if args.len() != 1 {
            return Err("help does not accept additional arguments".to_string());
        }
        return Ok(CliCommand::Help);
    }

    Ok(CliCommand::Run {
        roots: args.into_iter().map(PathBuf::from).collect(),
    })
}
