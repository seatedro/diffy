use std::sync::Arc;

use crate::contracts::viewport::interaction::{InteractionFailure, SearchMatch, SearchRequest};

#[derive(Debug, Clone, Copy, Default)]
pub struct InteractionServices;

impl InteractionServices {
    pub fn search(
        &self,
        request: &SearchRequest,
    ) -> Result<Arc<[SearchMatch]>, InteractionFailure> {
        tracing::debug!(
            job = request.job.0,
            documents = request.documents.len(),
            "search: not yet ported"
        );
        Err(InteractionFailure::NotPorted)
    }
}
