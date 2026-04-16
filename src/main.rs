use std::io::IsTerminal;
use std::path::PathBuf;

use artix::scan::scan_workspace;
use artix::ui::{build_overview_rows, run_tui};

fn main() {
    let roots: Vec<PathBuf> = std::env::args()
        .skip(1)
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    let roots = if roots.is_empty() {
        vec![std::env::current_dir().expect("current working directory must be readable")]
    } else {
        roots
    };

    if std::io::stdout().is_terminal() && std::env::var("ARTIX_PLAIN").is_err() {
        let start_dir = roots
            .first()
            .cloned()
            .expect("at least one root is always present");
        if let Err(err) = run_tui(start_dir) {
            eprintln!("artix: {err}");
            std::process::exit(1);
        }
        return;
    }

    let report = scan_workspace(&roots);
    let rows = build_overview_rows(report.projects);

    for row in rows {
        println!(
            "{}\t{}\t{}",
            row.project_name, row.reclaimable_bytes, row.candidate_count
        );
    }
}
