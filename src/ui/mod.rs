use crate::model::Project;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverviewRow {
    pub project_name: String,
    pub reclaimable_bytes: u64,
    pub candidate_count: usize,
}

pub fn build_overview_rows(mut projects: Vec<Project>) -> Vec<OverviewRow> {
    projects.sort_by(|left, right| right.reclaimable_bytes.cmp(&left.reclaimable_bytes));

    projects
        .into_iter()
        .map(|project| OverviewRow {
            project_name: project.name,
            reclaimable_bytes: project.reclaimable_bytes,
            candidate_count: project.candidate_count,
        })
        .collect()
}
