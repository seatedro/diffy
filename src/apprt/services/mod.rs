pub mod commands;
pub mod delivery;
pub mod mutations;
pub mod navigation;
pub mod review;
pub mod viewport;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::core::error::{DiffyError, Result};
use crate::core::vcs::backend::VcsRepository;
use crate::core::vcs::discovery;
use crate::core::vcs::git::is_pr_ref;
use crate::platform::persistence::SettingsStore;
use crate::platform::review_store::ReviewStore;

pub use commands::CommandsServices;
pub use delivery::DeliveryServices;
pub use mutations::MutationsServices;
pub use navigation::NavigationServices;
pub use review::{GitHubBackend, LiveGitHub, ReviewServices};
pub use viewport::ViewportServices;

#[derive(Clone)]
pub struct AppServices {
    pub shell: ShellServices,
    pub viewport: ViewportServices,
    pub navigation: NavigationServices,
    pub commands: CommandsServices,
    pub review: ReviewServices,
    pub mutations: MutationsServices,
    pub delivery: DeliveryServices,
}

impl AppServices {
    pub fn new(settings_store: SettingsStore) -> Self {
        let review_store = ReviewStore::for_settings_store(&settings_store);
        let github = Arc::new(LiveGitHub::new(review_store.clone()));
        Self::with_github(settings_store, review_store, github)
    }

    pub fn with_github(
        settings_store: SettingsStore,
        review_store: ReviewStore,
        github: Arc<dyn GitHubBackend>,
    ) -> Self {
        Self {
            shell: ShellServices,
            viewport: ViewportServices::default(),
            navigation: NavigationServices,
            commands: CommandsServices::new(settings_store.clone()),
            review: ReviewServices::new(settings_store, review_store, github),
            mutations: MutationsServices,
            delivery: DeliveryServices,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ShellServices;

impl ShellServices {
    pub fn open_repository_dialog(&self) -> Option<PathBuf> {
        rfd::FileDialog::new().pick_folder()
    }

    pub fn open_browser(&self, url: &str) -> Result<()> {
        webbrowser::open(url)
            .map(|_| ())
            .map_err(|error| DiffyError::General(format!("failed to open browser: {error}")))
    }

    pub fn set_clipboard(&self, text: String) {
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            let _ = clipboard.set_text(text);
        }
    }
}

/// PR comparison refs live in the git ref namespace (`refs/diffy/pr/...`),
/// which jj revsets cannot resolve. Route those comparisons through the git
/// backend, which in colocated jj checkouts operates on the same `.git` store.
pub(crate) fn open_compare_repository<'a>(
    repo_path: &Path,
    refs: impl IntoIterator<Item = &'a str>,
) -> Result<Box<dyn VcsRepository>> {
    if refs.into_iter().any(is_pr_ref) {
        discovery::open_git_repository(repo_path)
    } else {
        discovery::open_repository(repo_path)
    }
}
