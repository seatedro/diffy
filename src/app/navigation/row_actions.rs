use crate::app::mutations::RowAction;
use crate::contracts::shared::{FileKey, SharedContext};
use crate::contracts::viewport::pipeline::ManifestFile;
use crate::core::vcs::model::ChangeBucket;

pub fn file_row_actions(_file: &ManifestFile, _cx: &SharedContext) -> Vec<RowAction> {
    Vec::new()
}

pub fn group_action(_bucket: ChangeBucket, _cx: &SharedContext) -> Option<RowAction> {
    None
}

pub fn selection_actions(_files: &[FileKey], _cx: &SharedContext) -> Vec<RowAction> {
    Vec::new()
}
