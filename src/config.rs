use octocrab::models::WorkflowId;

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Config {
    #[serde(default)]
    pub github: Github,
}

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Github {
    pub update_snapshot_workflow_name: Option<WorkflowId>,
}
