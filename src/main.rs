use std::io::IsTerminal;
use std::path::PathBuf;

use artix::config::{AppContext, UiMode, load_config};
use artix::scan::scan_workspace_with_context;
use artix::ui::{build_overview_rows, run_tui_with_context};

#[tokio::main]
async fn main() {
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

    let roots: Vec<PathBuf> = std::env::args()
        .skip(1)
        .map(PathBuf::from)
        .collect::<Vec<_>>();
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
