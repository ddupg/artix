use crate::model::RiskLevel;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub id: &'static str,
    pub kind: &'static str,
    pub dir_name: &'static str,
    pub language_hint: &'static str,
    pub default_risk: RiskLevel,
    pub expect_git_ignored: bool,
}

pub fn default_rules() -> Vec<Rule> {
    vec![
        Rule {
            id: "rust.target",
            kind: "rust-target",
            dir_name: "target",
            language_hint: "rust",
            default_risk: RiskLevel::Low,
            expect_git_ignored: true,
        },
        Rule {
            id: "node.node_modules",
            kind: "node-modules",
            dir_name: "node_modules",
            language_hint: "node",
            default_risk: RiskLevel::Medium,
            expect_git_ignored: true,
        },
        Rule {
            id: "python.venv",
            kind: "python-venv",
            dir_name: ".venv",
            language_hint: "python",
            default_risk: RiskLevel::Medium,
            expect_git_ignored: true,
        },
    ]
}
