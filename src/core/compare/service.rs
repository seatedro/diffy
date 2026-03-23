use crate::core::compare::backends::{DiffBackend, DifftasticBackend, GitDiffBackend};
use crate::core::compare::spec::{CompareSpec, RendererKind};
use crate::core::diff::FileDiff;
use crate::core::error::{DiffyError, Result};
use crate::core::text::{TextBuffer, TokenBuffer};
use crate::core::vcs::git::GitService;

#[derive(Debug, Default)]
pub struct CompareOutput {
    pub files: Vec<FileDiff>,
    pub raw_diff: String,
    pub text_buffer: TextBuffer,
    pub token_buffer: TokenBuffer,
    pub used_fallback: bool,
    pub fallback_message: String,
}

pub struct CompareService {
    primary: Box<dyn DiffBackend>,
    fallback: Box<dyn DiffBackend>,
}

impl Default for CompareService {
    fn default() -> Self {
        Self {
            primary: Box::new(DifftasticBackend),
            fallback: Box::new(GitDiffBackend),
        }
    }
}

impl CompareService {
    pub fn compare(&self, spec: &CompareSpec, git: &GitService) -> Result<CompareOutput> {
        if spec.renderer == RendererKind::Builtin {
            return self.fallback.compare(spec, git)?.ok_or_else(|| {
                DiffyError::General("built-in backend returned no result".to_owned())
            });
        }

        match self.primary.compare(spec, git)? {
            Some(output) => Ok(output),
            None => {
                let mut fallback = self.fallback.compare(spec, git)?.ok_or_else(|| {
                    DiffyError::General("fallback backend returned no result".to_owned())
                })?;
                fallback.used_fallback = true;
                fallback.fallback_message =
                    "difftastic unavailable, fell back to built-in backend".to_owned();
                Ok(fallback)
            }
        }
    }
}
