#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineAction {
    Noop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineEffect {
    Noop,
}

#[derive(Debug, Clone)]
pub enum PipelineEvent {
    Noop,
}

#[derive(Debug, Clone, Default)]
pub struct CompareManifestSnapshot {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareIntent {}

#[derive(Debug, Clone)]
pub struct PreparedDiff {}
