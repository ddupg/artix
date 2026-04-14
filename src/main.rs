use std::path::PathBuf;

use artix::scan::scan_workspace;
use artix::ui::build_overview_rows;

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
    let report = scan_workspace(&roots);
    let rows = build_overview_rows(report.projects);

    for row in rows {
        println!(
            "{}\t{}\t{}",
            row.project_name, row.reclaimable_bytes, row.candidate_count
        );
    }
}
