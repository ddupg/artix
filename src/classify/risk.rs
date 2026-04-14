use crate::model::{GitStatus, RiskLevel};
use crate::rules::Rule;

pub fn classify_risk_level(rule: &Rule, _git_status: &GitStatus) -> RiskLevel {
    rule.default_risk.clone()
}
