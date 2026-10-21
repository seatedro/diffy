use std::sync::Arc;

use diffy_diff::{DiffDocument, DocumentBuilder, FileMeta, FileStatus, SourceCoverage};

use crate::apprt::services::open_compare_repository;
use crate::contracts::viewport::files::FileLoadRequest;
use crate::core::vcs::source::SourceText;

#[derive(Debug, Clone, Copy, Default)]
pub struct FilesServices;

impl FilesServices {
    pub fn load(&self, request: &FileLoadRequest) -> Result<Arc<DiffDocument>, String> {
        let file = &request.file;
        let mut repo = open_compare_repository(&request.repo_path, [file.revision.id.as_str()])
            .map_err(|error| error.user_message())?;
        let source = repo
            .read_file_source(&file.revision, &file.path)
            .map_err(|error| error.user_message())?;
        let SourceText::Text(store) = source else {
            return Err("Binary file".to_owned());
        };
        let mut builder = DocumentBuilder::new();
        builder
            .file(
                FileMeta {
                    old_path: Some(file.path.as_str().into()),
                    new_path: Some(file.path.as_str().into()),
                    status: FileStatus::Modified,
                    ..FileMeta::default()
                },
                SourceCoverage::Full,
                store.clone(),
                store,
            )
            .finish()
            .map_err(|error| error.to_string())?;
        Ok(Arc::new(builder.build()))
    }
}
