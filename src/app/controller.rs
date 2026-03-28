use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use qmetaobject::*;
use qttypes::QSettings;

use crate::app::models::{DiffRowItem, RepositoryPickerModel};
use crate::app::qt_types::{
    branch_infos_to_qvariant_list, commit_infos_to_qvariant_list, file_diff_to_qvariant_map,
    file_diffs_to_qvariant_list, pull_request_info_to_qvariant_map, tag_infos_to_qvariant_list,
};
use crate::core::compare::backends::DifftasticBackend;
use crate::core::compare::{CompareMode, CompareService, CompareSpec, LayoutMode, RendererKind};
use crate::core::diff::{DiffLine, FileDiff};
use crate::core::rendering::{DiffRowType, FlatDiffRow, flatten_file_diff};
use crate::core::search::fuzzy::fuzzy_score;
use crate::core::text::{TextBuffer, TokenBuffer};
use crate::core::vcs::git::GitService;
use crate::core::vcs::github::{GitHubApi, parse_pr_url, poll_for_token, start_device_flow};

struct CompareResult {
    files: Vec<FileDiff>,
    text_buffer: TextBuffer,
    token_buffer: TokenBuffer,
    used_fallback: bool,
    fallback_message: String,
    error: String,
}

impl Default for CompareResult {
    fn default() -> Self {
        Self {
            files: Vec::new(),
            text_buffer: TextBuffer::default(),
            token_buffer: TokenBuffer::default(),
            used_fallback: false,
            fallback_message: String::new(),
            error: String::new(),
        }
    }
}

#[derive(Default)]
struct PullRequestResult {
    info: Option<crate::core::vcs::github::PullRequestInfo>,
    left_ref: String,
    right_ref: String,
    error: String,
}

#[allow(non_snake_case)]
#[derive(QObject)]
pub struct DiffController {
    base: qt_base_class!(trait QObject),

    current_view: qt_property!(QString; READ get_current_view NOTIFY currentViewChanged ALIAS currentView),
    currentViewChanged: qt_signal!(),

    recent_repositories: qt_property!(QVariantList; READ get_recent_repositories NOTIFY recent_repositories_changed ALIAS recentRepositories),
    recent_repositories_changed: qt_signal!(),

    repo_path: qt_property!(QString; READ get_repo_path WRITE set_repo_path NOTIFY repo_path_changed ALIAS repoPath),
    repo_path_changed: qt_signal!(),

    refs: qt_property!(QVariantList; READ get_refs NOTIFY refs_changed ALIAS refs),
    refs_changed: qt_signal!(),

    left_ref: qt_property!(QString; READ get_left_ref WRITE set_left_ref NOTIFY leftRefChanged ALIAS leftRef),
    leftRefChanged: qt_signal!(),

    right_ref: qt_property!(QString; READ get_right_ref WRITE set_right_ref NOTIFY rightRefChanged ALIAS rightRef),
    rightRefChanged: qt_signal!(),

    left_ref_display: qt_property!(QString; READ get_left_ref_display NOTIFY leftRefChanged ALIAS leftRefDisplay),
    right_ref_display: qt_property!(QString; READ get_right_ref_display NOTIFY rightRefChanged ALIAS rightRefDisplay),

    compare_mode: qt_property!(QString; READ get_compare_mode WRITE set_compare_mode NOTIFY compare_mode_changed ALIAS compareMode),
    compare_mode_changed: qt_signal!(),

    renderer: qt_property!(QString; READ get_renderer WRITE set_renderer NOTIFY renderer_changed ALIAS renderer),
    renderer_changed: qt_signal!(),

    layout_mode: qt_property!(QString; READ get_layout_mode WRITE set_layout_mode NOTIFY layout_mode_changed ALIAS layoutMode),
    layout_mode_changed: qt_signal!(),

    compare_generation: qt_property!(i32; READ get_compare_generation NOTIFY compare_generation_changed ALIAS compareGeneration),
    compare_generation_changed: qt_signal!(),

    files: qt_property!(QVariantList; READ get_files NOTIFY files_changed ALIAS files),
    files_changed: qt_signal!(),

    selected_file_index: qt_property!(i32; READ get_selected_file_index WRITE set_selected_file_index NOTIFY selectedFileIndexChanged ALIAS selectedFileIndex),
    selectedFileIndexChanged: qt_signal!(),

    selected_file: qt_property!(QVariantMap; READ get_selected_file NOTIFY selected_file_changed ALIAS selectedFile),
    selected_file_changed: qt_signal!(),

    selected_file_rows_model: qt_property!(RefCell<SimpleListModel<DiffRowItem>>; CONST ALIAS selectedFileRowsModel),
    selected_file_row_count: qt_property!(i32; READ get_selected_file_row_count NOTIFY selected_file_rows_changed ALIAS selectedFileRowCount),
    selected_file_rows_changed: qt_signal!(),

    repository_picker_visible: qt_property!(bool; READ get_repository_picker_visible NOTIFY repository_picker_visible_changed ALIAS repositoryPickerVisible),
    repository_picker_visible_changed: qt_signal!(),

    repository_picker_model: qt_property!(RefCell<RepositoryPickerModel>; CONST ALIAS repositoryPickerModel),

    branches: qt_property!(QVariantList; READ get_branches NOTIFY branches_changed ALIAS branches),
    branches_changed: qt_signal!(),

    tags: qt_property!(QVariantList; READ get_tags NOTIFY tags_changed ALIAS tags),
    tags_changed: qt_signal!(),

    commits: qt_property!(QVariantList; READ get_commits NOTIFY commits_changed ALIAS commits),
    commits_changed: qt_signal!(),

    pull_request_info: qt_property!(QVariantMap; READ get_pull_request_info NOTIFY pull_request_info_changed ALIAS pullRequestInfo),
    pull_request_info_changed: qt_signal!(),

    comparing: qt_property!(bool; READ get_comparing NOTIFY comparing_changed ALIAS comparing),
    comparing_changed: qt_signal!(),

    pull_request_loading: qt_property!(bool; READ get_pull_request_loading NOTIFY pull_request_loading_changed ALIAS pullRequestLoading),
    pull_request_loading_changed: qt_signal!(),

    github_token: qt_property!(QString; READ get_github_token WRITE set_github_token NOTIFY github_token_changed ALIAS githubToken),
    github_token_changed: qt_signal!(),

    has_github_token: qt_property!(bool; READ get_has_github_token NOTIFY github_token_changed ALIAS hasGithubToken),

    oauth_in_progress: qt_property!(bool; READ get_oauth_in_progress NOTIFY oauthStateChanged ALIAS oauthInProgress),
    oauth_user_code: qt_property!(QString; READ get_oauth_user_code NOTIFY oauthStateChanged ALIAS oauthUserCode),
    oauth_verification_uri: qt_property!(QString; READ get_oauth_verification_uri NOTIFY oauthStateChanged ALIAS oauthVerificationUri),
    oauthStateChanged: qt_signal!(),

    error_message: qt_property!(QString; READ get_error_message NOTIFY error_message_changed ALIAS errorMessage),
    error_message_changed: qt_signal!(),

    wrap_enabled: qt_property!(bool; READ get_wrap_enabled WRITE set_wrap_enabled NOTIFY wrap_enabled_changed ALIAS wrapEnabled),
    wrap_enabled_changed: qt_signal!(),

    wrap_column: qt_property!(i32; READ get_wrap_column WRITE set_wrap_column NOTIFY wrap_column_changed ALIAS wrapColumn),
    wrap_column_changed: qt_signal!(),

    has_difftastic: qt_property!(bool; READ get_has_difftastic NOTIFY has_difftastic_changed ALIAS hasDifftastic),
    has_difftastic_changed: qt_signal!(),

    go_back: qt_method!(fn(&mut self)),
    open_repository: qt_method!(fn(&mut self, path: QString) -> bool),
    open_repository_picker: qt_method!(fn(&mut self)),
    open_repository_from_dialog: qt_method!(fn(&mut self)),
    close_repository_picker: qt_method!(fn(&mut self)),
    navigate_repository_picker_up: qt_method!(fn(&mut self)),
    activate_repository_picker_entry: qt_method!(fn(&mut self, index: i32)),
    open_current_repository_from_picker: qt_method!(fn(&mut self)),
    compare: qt_method!(fn(&mut self)),
    select_file: qt_method!(fn(&mut self, index: i32)),
    load_branches: qt_method!(fn(&mut self)),
    load_tags: qt_method!(fn(&mut self)),
    load_commits: qt_method!(fn(&mut self, ref_name: QString)),
    search_commits: qt_method!(fn(&self, hex_prefix: QString) -> QVariantList),
    record_recent_branch: qt_method!(fn(&mut self, name: QString)),
    recent_branches_for_repo: qt_method!(fn(&self) -> QVariantList),
    open_pull_request: qt_method!(fn(&mut self, url: QString)),
    fuzzy_filter: qt_method!(
        fn(&self, query: QString, items: QVariantList, label_key: QString) -> QVariantList
    ),
    start_oauth_login: qt_method!(fn(&mut self)),
    cancel_oauth_login: qt_method!(fn(&mut self)),
    copy_to_clipboard: qt_method!(fn(&self, text: QString)),

    settings: QSettings,
    git_service: GitService,
    _compare_service: CompareService,
    file_diffs_store: Vec<FileDiff>,
    text_buffer_store: TextBuffer,
    token_buffer_store: TokenBuffer,
    oauth_generation: Arc<AtomicU64>,
}

impl Default for DiffController {
    fn default() -> Self {
        let settings = QSettings::from_path(&settings_file_path("controller.ini"));
        let recent = decode_string_list(&settings.value_string("recentRepositories"));
        let github_token = {
            let saved = settings.value_string("githubToken");
            if saved.is_empty() {
                std::env::var("GITHUB_TOKEN").unwrap_or_default()
            } else {
                saved
            }
        };

        let mut this = Self {
            base: Default::default(),
            current_view: QString::from("welcome"),
            currentViewChanged: Default::default(),
            recent_repositories: recent
                .iter()
                .map(|s| QVariant::from(QString::from(s.as_str())))
                .collect(),
            recent_repositories_changed: Default::default(),
            repo_path: QString::from(settings.value_string("repoPath")),
            repo_path_changed: Default::default(),
            refs: QVariantList::default(),
            refs_changed: Default::default(),
            left_ref: QString::from(settings.value_string("leftRef")),
            leftRefChanged: Default::default(),
            right_ref: QString::from(settings.value_string("rightRef")),
            rightRefChanged: Default::default(),
            left_ref_display: Default::default(),
            right_ref_display: Default::default(),
            compare_mode: QString::from(non_empty(settings.value_string("compareMode"), "two-dot")),
            compare_mode_changed: Default::default(),
            renderer: QString::from(non_empty(settings.value_string("renderer"), "builtin")),
            renderer_changed: Default::default(),
            layout_mode: QString::from(non_empty(settings.value_string("layoutMode"), "unified")),
            layout_mode_changed: Default::default(),
            compare_generation: 0,
            compare_generation_changed: Default::default(),
            files: QVariantList::default(),
            files_changed: Default::default(),
            selected_file_index: -1,
            selectedFileIndexChanged: Default::default(),
            selected_file: QVariantMap::default(),
            selected_file_changed: Default::default(),
            selected_file_rows_model: RefCell::new(SimpleListModel::default()),
            selected_file_row_count: 0,
            selected_file_rows_changed: Default::default(),
            repository_picker_visible: false,
            repository_picker_visible_changed: Default::default(),
            repository_picker_model: RefCell::new(RepositoryPickerModel::default()),
            branches: QVariantList::default(),
            branches_changed: Default::default(),
            tags: QVariantList::default(),
            tags_changed: Default::default(),
            commits: QVariantList::default(),
            commits_changed: Default::default(),
            pull_request_info: QVariantMap::default(),
            pull_request_info_changed: Default::default(),
            comparing: false,
            comparing_changed: Default::default(),
            pull_request_loading: false,
            pull_request_loading_changed: Default::default(),
            github_token: QString::from(github_token),
            github_token_changed: Default::default(),
            has_github_token: false,
            oauth_in_progress: false,
            oauth_user_code: QString::default(),
            oauth_verification_uri: QString::default(),
            oauthStateChanged: Default::default(),
            error_message: QString::default(),
            error_message_changed: Default::default(),
            wrap_enabled: settings.value_bool("wrapEnabled"),
            wrap_enabled_changed: Default::default(),
            wrap_column: settings.value_string("wrapColumn").parse().unwrap_or(0),
            wrap_column_changed: Default::default(),
            has_difftastic: DifftasticBackend::is_available(),
            has_difftastic_changed: Default::default(),
            go_back: Default::default(),
            open_repository: Default::default(),
            open_repository_picker: Default::default(),
            open_repository_from_dialog: Default::default(),
            close_repository_picker: Default::default(),
            navigate_repository_picker_up: Default::default(),
            activate_repository_picker_entry: Default::default(),
            open_current_repository_from_picker: Default::default(),
            compare: Default::default(),
            select_file: Default::default(),
            load_branches: Default::default(),
            load_tags: Default::default(),
            load_commits: Default::default(),
            search_commits: Default::default(),
            record_recent_branch: Default::default(),
            recent_branches_for_repo: Default::default(),
            open_pull_request: Default::default(),
            fuzzy_filter: Default::default(),
            start_oauth_login: Default::default(),
            cancel_oauth_login: Default::default(),
            copy_to_clipboard: Default::default(),
            settings,
            git_service: GitService::new(),
            _compare_service: CompareService::default(),
            file_diffs_store: Vec::new(),
            text_buffer_store: TextBuffer::default(),
            token_buffer_store: TokenBuffer::default(),
            oauth_generation: Arc::new(AtomicU64::new(0)),
        };
        this.git_service
            .set_github_token(this.github_token.to_string());
        if !this.repo_path.to_string().is_empty() {
            let repo = this.repo_path.clone();
            let _ = this.open_repository(repo);
        }
        this
    }
}

impl DiffController {
    pub fn get_current_view(&self) -> QString {
        self.current_view.clone()
    }
    pub fn get_recent_repositories(&self) -> QVariantList {
        self.recent_repositories.clone()
    }
    pub fn get_repo_path(&self) -> QString {
        self.repo_path.clone()
    }
    pub fn get_refs(&self) -> QVariantList {
        self.refs.clone()
    }
    pub fn get_left_ref(&self) -> QString {
        self.left_ref.clone()
    }
    pub fn get_right_ref(&self) -> QString {
        self.right_ref.clone()
    }
    pub fn get_left_ref_display(&self) -> QString {
        QString::from(self.abbreviate_ref(&self.left_ref.to_string()))
    }
    pub fn get_right_ref_display(&self) -> QString {
        QString::from(self.abbreviate_ref(&self.right_ref.to_string()))
    }
    pub fn get_compare_mode(&self) -> QString {
        self.compare_mode.clone()
    }
    pub fn get_renderer(&self) -> QString {
        self.renderer.clone()
    }
    pub fn get_layout_mode(&self) -> QString {
        self.layout_mode.clone()
    }
    pub fn get_compare_generation(&self) -> i32 {
        self.compare_generation
    }
    pub fn get_files(&self) -> QVariantList {
        self.files.clone()
    }
    pub fn get_selected_file_index(&self) -> i32 {
        self.selected_file_index
    }
    pub fn get_selected_file(&self) -> QVariantMap {
        if self.selected_file_index < 0
            || self.selected_file_index as usize >= self.file_diffs_store.len()
        {
            QVariantMap::default()
        } else {
            file_diff_to_qvariant_map(&self.file_diffs_store[self.selected_file_index as usize])
        }
    }
    pub fn get_selected_file_row_count(&self) -> i32 {
        self.selected_file_rows_model.borrow().iter().count() as i32
    }
    pub fn get_repository_picker_visible(&self) -> bool {
        self.repository_picker_visible
    }
    pub fn get_branches(&self) -> QVariantList {
        self.branches.clone()
    }
    pub fn get_tags(&self) -> QVariantList {
        self.tags.clone()
    }
    pub fn get_commits(&self) -> QVariantList {
        self.commits.clone()
    }
    pub fn get_pull_request_info(&self) -> QVariantMap {
        self.pull_request_info.clone()
    }
    pub fn get_comparing(&self) -> bool {
        self.comparing
    }
    pub fn get_pull_request_loading(&self) -> bool {
        self.pull_request_loading
    }
    pub fn get_github_token(&self) -> QString {
        self.github_token.clone()
    }
    pub fn get_has_github_token(&self) -> bool {
        !self.github_token.to_string().is_empty()
    }
    pub fn get_oauth_in_progress(&self) -> bool {
        !self.oauth_user_code.to_string().is_empty()
    }
    pub fn get_oauth_user_code(&self) -> QString {
        self.oauth_user_code.clone()
    }
    pub fn get_oauth_verification_uri(&self) -> QString {
        self.oauth_verification_uri.clone()
    }
    pub fn get_error_message(&self) -> QString {
        self.error_message.clone()
    }
    pub fn get_wrap_enabled(&self) -> bool {
        self.wrap_enabled
    }
    pub fn get_wrap_column(&self) -> i32 {
        self.wrap_column
    }
    pub fn get_has_difftastic(&self) -> bool {
        self.has_difftastic
    }

    pub fn set_repo_path(&mut self, path: QString) {
        if self.repo_path == path {
            return;
        }
        self.repo_path = path;
        self.repo_path_changed();
    }
    pub fn set_left_ref(&mut self, value: QString) {
        if self.left_ref == value {
            return;
        }
        self.left_ref = value.clone();
        if !looks_like_oid(&value.to_string()) {
            self.record_recent_branch(value);
        }
        self.leftRefChanged();
    }
    pub fn set_right_ref(&mut self, value: QString) {
        if self.right_ref == value {
            return;
        }
        self.right_ref = value.clone();
        if !looks_like_oid(&value.to_string()) {
            self.record_recent_branch(value);
        }
        self.rightRefChanged();
    }
    pub fn set_compare_mode(&mut self, value: QString) {
        if self.compare_mode == value {
            return;
        }
        self.compare_mode = value;
        self.compare_mode_changed();
    }
    pub fn set_renderer(&mut self, value: QString) {
        if self.renderer == value {
            return;
        }
        self.renderer = value;
        self.renderer_changed();
    }
    pub fn set_layout_mode(&mut self, value: QString) {
        if self.layout_mode == value {
            return;
        }
        self.layout_mode = value;
        self.layout_mode_changed();
    }
    pub fn set_selected_file_index(&mut self, index: i32) {
        if self.selected_file_index == index {
            return;
        }
        self.selected_file_index = index;
        self.rebuild_selected_file_rows();
        self.selectedFileIndexChanged();
        self.selected_file_changed();
    }
    pub fn set_github_token(&mut self, token: QString) {
        let trimmed = token.to_string().trim().to_owned();
        if self.github_token.to_string() == trimmed {
            return;
        }
        self.github_token = QString::from(trimmed.clone());
        self.git_service.set_github_token(trimmed.clone());
        self.settings.set_string("githubToken", &trimmed);
        self.settings.sync();
        self.github_token_changed();
    }
    pub fn set_wrap_enabled(&mut self, value: bool) {
        if self.wrap_enabled == value {
            return;
        }
        self.wrap_enabled = value;
        self.settings.set_bool("wrapEnabled", value);
        self.settings.sync();
        self.wrap_enabled_changed();
    }
    pub fn set_wrap_column(&mut self, value: i32) {
        if self.wrap_column == value {
            return;
        }
        self.wrap_column = value;
        self.settings.set_string("wrapColumn", &value.to_string());
        self.settings.sync();
        self.wrap_column_changed();
    }

    pub fn go_back(&mut self) {
        let next = match self.current_view.to_string().as_str() {
            "diff" => "compare",
            "compare" => "welcome",
            _ => "welcome",
        };
        self.set_current_view(next);
    }

    pub fn open_repository(&mut self, path: QString) -> bool {
        self.clear_error();
        let path_str = path.to_string();
        if path_str.is_empty() {
            self.set_error("Repository path is empty");
            return false;
        }
        if let Err(error) = self.git_service.open(&path_str) {
            self.set_error(error.to_string());
            return false;
        }
        let changed = self.repo_path.to_string() != path_str;
        self.repo_path = path;
        self.repo_path_changed();

        match self.git_service.refs() {
            Ok(refs) => {
                self.refs = refs
                    .into_iter()
                    .map(|s| QVariant::from(QString::from(s.as_str())))
                    .collect();
                self.refs_changed();
            }
            Err(error) => self.set_error(error.to_string()),
        }

        if changed {
            self.compare_generation += 1;
            self.compare_generation_changed();
            self.file_diffs_store.clear();
            self.text_buffer_store.clear();
            self.token_buffer_store.clear();
            self.files = QVariantList::default();
            self.files_changed();
            self.selected_file_index = -1;
            self.rebuild_selected_file_rows();
            self.selectedFileIndexChanged();
            self.selected_file_changed();
        }

        if self.refs.len() > 0 {
            if changed || self.left_ref.to_string().is_empty() {
                self.left_ref = self.refs[0].to_qstring();
                self.leftRefChanged();
            }
            if changed || self.right_ref.to_string().is_empty() {
                let idx = if self.refs.len() > 1 { 1 } else { 0 };
                self.right_ref = self.refs[idx].to_qstring();
                self.rightRefChanged();
            }
        }

        self.add_recent_repository(&path_str);
        self.load_branches();
        self.set_current_view("compare");
        self.persist_settings();
        true
    }

    pub fn open_repository_picker(&mut self) {
        let start = if self.repo_path.to_string().is_empty() {
            std::env::var("HOME").unwrap_or_else(|_| "/".to_owned())
        } else {
            self.repo_path.to_string()
        };
        self.repository_picker_model
            .borrow_mut()
            .set_current_path(QString::from(start));
        if !self.repository_picker_visible {
            self.repository_picker_visible = true;
            self.repository_picker_visible_changed();
        }
    }

    pub fn open_repository_from_dialog(&mut self) {
        self.open_repository_picker();
    }

    pub fn close_repository_picker(&mut self) {
        if !self.repository_picker_visible {
            return;
        }
        self.repository_picker_visible = false;
        self.repository_picker_visible_changed();
    }

    pub fn navigate_repository_picker_up(&mut self) {
        self.repository_picker_model.borrow_mut().go_up();
    }

    pub fn activate_repository_picker_entry(&mut self, index: i32) {
        if self
            .repository_picker_model
            .borrow()
            .entry_is_repository(index)
        {
            let path = self.repository_picker_model.borrow().entry_path(index);
            if self.open_repository(path) {
                self.close_repository_picker();
            }
            return;
        }
        let _ = self
            .repository_picker_model
            .borrow_mut()
            .navigate_to_entry(index);
    }

    pub fn open_current_repository_from_picker(&mut self) {
        if !self
            .repository_picker_model
            .borrow()
            .get_current_path_is_repository()
        {
            return;
        }
        let path = self.repository_picker_model.borrow().get_current_path();
        if self.open_repository(path) {
            self.close_repository_picker();
        }
    }

    pub fn compare(&mut self) {
        self.clear_error();
        if !self.git_service.is_open() {
            self.set_error("Open a repository before running compare");
            return;
        }
        let mode = self.compare_mode.to_string().parse::<CompareMode>();
        let renderer = self.renderer.to_string().parse::<RendererKind>();
        let layout = self.layout_mode.to_string().parse::<LayoutMode>();
        let (Ok(mode), Ok(renderer), Ok(layout)) = (mode, renderer, layout) else {
            self.set_error("Invalid compare configuration");
            return;
        };
        let spec = CompareSpec {
            mode,
            left_ref: self.left_ref.to_string(),
            right_ref: self.right_ref.to_string(),
            renderer,
            layout,
        };
        let repo_path = self.repo_path.to_string();
        let token = self.github_token.to_string();
        self.comparing = true;
        self.comparing_changed();
        let qptr = QPointer::from(&*self);
        let callback = queued_callback(move |result: CompareResult| {
            if let Some(pinned) = qptr.as_pinned() {
                let mut this = pinned.borrow_mut();
                this.comparing = false;
                this.comparing_changed();
                if !result.error.is_empty() {
                    this.set_error(result.error);
                    return;
                }
                if result.used_fallback && !result.fallback_message.is_empty() {
                    this.set_error(result.fallback_message);
                }
                this.compare_generation += 1;
                this.compare_generation_changed();
                this.file_diffs_store = result.files;
                this.text_buffer_store = result.text_buffer;
                this.token_buffer_store = result.token_buffer;
                this.files = file_diffs_to_qvariant_list(&this.file_diffs_store);
                this.files_changed();
                this.selected_file_index = if this.file_diffs_store.is_empty() {
                    -1
                } else {
                    0
                };
                this.selectedFileIndexChanged();
                this.selected_file_changed();
                this.rebuild_selected_file_rows();
                this.persist_settings();
                this.set_current_view("diff");
            }
        });
        thread::spawn(move || {
            let mut result = CompareResult::default();
            let mut git = GitService::new();
            git.set_github_token(token);
            match git
                .open(&repo_path)
                .and_then(|_| CompareService::default().compare(&spec, &git))
            {
                Ok(output) => {
                    result.files = output.files;
                    result.text_buffer = output.text_buffer;
                    result.token_buffer = output.token_buffer;
                    result.used_fallback = output.used_fallback;
                    result.fallback_message = output.fallback_message;
                }
                Err(error) => result.error = error.to_string(),
            }
            callback(result);
        });
    }

    pub fn select_file(&mut self, index: i32) {
        self.set_selected_file_index(index);
    }

    pub fn load_branches(&mut self) {
        match self.git_service.branches() {
            Ok(branches) => {
                self.branches = branch_infos_to_qvariant_list(&branches);
                self.branches_changed();
            }
            Err(error) => self.set_error(error.to_string()),
        }
        self.load_tags();
    }

    pub fn load_tags(&mut self) {
        match self.git_service.tags() {
            Ok(tags) => {
                self.tags = tag_infos_to_qvariant_list(&tags);
                self.tags_changed();
            }
            Err(error) => self.set_error(error.to_string()),
        }
    }

    pub fn load_commits(&mut self, ref_name: QString) {
        match self.git_service.commits(&ref_name.to_string(), 100) {
            Ok(commits) => {
                self.commits = commit_infos_to_qvariant_list(&commits);
                self.commits_changed();
            }
            Err(error) => self.set_error(error.to_string()),
        }
    }

    pub fn search_commits(&self, hex_prefix: QString) -> QVariantList {
        self.git_service
            .search_commits(&hex_prefix.to_string())
            .map(|commits| commit_infos_to_qvariant_list(&commits))
            .unwrap_or_default()
    }

    pub fn record_recent_branch(&mut self, name: QString) {
        let repo = self.repo_path.to_string();
        if repo.is_empty() {
            return;
        }
        let key = format!("recentBranches/{repo}");
        let mut branches = decode_string_list(&self.settings.value_string(&key));
        let name = name.to_string();
        branches.retain(|item| item != &name);
        branches.insert(0, name);
        branches.truncate(8);
        self.settings
            .set_string(&key, &encode_string_list(&branches));
        self.settings.sync();
    }

    pub fn recent_branches_for_repo(&self) -> QVariantList {
        let repo = self.repo_path.to_string();
        if repo.is_empty() {
            return QVariantList::default();
        }
        let key = format!("recentBranches/{repo}");
        decode_string_list(&self.settings.value_string(&key))
            .into_iter()
            .map(|item| {
                let map: QVariantMap = [(
                    QString::from("name"),
                    QVariant::from(QString::from(item.as_str())),
                )]
                .into_iter()
                .collect();
                QVariant::from(map)
            })
            .collect()
    }

    pub fn open_pull_request(&mut self, url: QString) {
        self.clear_error();
        let url_string = url.to_string();
        let Some(parsed) = parse_pr_url(&url_string) else {
            self.set_error("Not a valid GitHub pull request URL");
            self.pull_request_info = QVariantMap::default();
            self.pull_request_info_changed();
            return;
        };
        self.pull_request_loading = true;
        self.pull_request_loading_changed();
        let repo_path = self.repo_path.to_string();
        let token = self.github_token.to_string();
        let qptr = QPointer::from(&*self);
        let callback = queued_callback(move |result: PullRequestResult| {
            if let Some(pinned) = qptr.as_pinned() {
                let mut this = pinned.borrow_mut();
                this.pull_request_loading = false;
                this.pull_request_loading_changed();
                if !result.error.is_empty() {
                    this.pull_request_info = QVariantMap::default();
                    this.pull_request_info_changed();
                    this.set_error(result.error);
                    return;
                }
                if let Some(info) = result.info.as_ref() {
                    this.pull_request_info = pull_request_info_to_qvariant_map(info);
                    this.pull_request_info_changed();
                }
                if !result.left_ref.is_empty() {
                    this.left_ref = QString::from(result.left_ref.as_str());
                    this.leftRefChanged();
                }
                if !result.right_ref.is_empty() {
                    this.right_ref = QString::from(result.right_ref.as_str());
                    this.rightRefChanged();
                }
                if this.compare_mode.to_string() != "three-dot" {
                    this.compare_mode = QString::from("three-dot");
                    this.compare_mode_changed();
                }
                if this.git_service.is_open() {
                    this.compare();
                }
            }
        });
        thread::spawn(move || {
            let mut result = PullRequestResult::default();
            let api = GitHubApi::with_token(token.clone());
            match api.fetch_pull_request(&parsed.owner, &parsed.repo, parsed.number) {
                Ok(info) => {
                    result.info = Some(info);
                    if !repo_path.is_empty() {
                        let mut git = GitService::new();
                        git.set_github_token(token);
                        match git
                            .open(&repo_path)
                            .and_then(|_| git.resolve_pull_request_comparison(&url_string))
                        {
                            Ok((left, right)) => {
                                result.left_ref = left;
                                result.right_ref = right;
                            }
                            Err(error) => result.error = error.to_string(),
                        }
                    }
                }
                Err(error) => result.error = error.to_string(),
            }
            callback(result);
        });
    }

    pub fn fuzzy_filter(
        &self,
        query: QString,
        items: QVariantList,
        label_key: QString,
    ) -> QVariantList {
        if query.to_string().is_empty() {
            return items;
        }
        let label_key = label_key.to_string();
        let entries = (&items).into_iter().cloned().collect::<Vec<_>>();
        let mut ranked = entries
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                let map = item.to_qvariantmap();
                let label = map
                    .value(QString::from(label_key.as_str()), QVariant::default())
                    .to_qstring()
                    .to_string();
                fuzzy_score(&query.to_string(), &label).map(|score| (index, score))
            })
            .collect::<Vec<_>>();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        ranked
            .into_iter()
            .map(|(index, _)| entries[index].clone())
            .collect()
    }

    pub fn start_oauth_login(&mut self) {
        self.clear_error();
        let client_id = std::env::var("DIFFY_GITHUB_CLIENT_ID")
            .unwrap_or_else(|_| "Ov23lijXMwtY1XmHedUM".to_owned());
        if client_id.is_empty() {
            self.set_error("Set DIFFY_GITHUB_CLIENT_ID first.");
            return;
        }
        match start_device_flow(&client_id) {
            Ok(state) => {
                self.oauth_user_code = QString::from(state.user_code.as_str());
                self.oauth_verification_uri = QString::from(state.verification_uri.as_str());
                self.oauthStateChanged();
                let generation = self.oauth_generation.fetch_add(1, Ordering::SeqCst) + 1;
                let shared_generation = Arc::clone(&self.oauth_generation);
                let qptr = QPointer::from(&*self);
                let callback = queued_callback(move |payload: (bool, String)| {
                    if let Some(pinned) = qptr.as_pinned() {
                        let mut this = pinned.borrow_mut();
                        this.oauth_user_code = QString::default();
                        this.oauth_verification_uri = QString::default();
                        this.oauthStateChanged();
                        if payload.0 {
                            this.set_github_token(QString::from(payload.1.as_str()));
                        } else if !payload.1.is_empty() {
                            this.set_error(payload.1);
                        }
                    }
                });
                thread::spawn(move || {
                    loop {
                        if shared_generation.load(Ordering::SeqCst) != generation {
                            return;
                        }
                        match poll_for_token(&client_id, &state.device_code) {
                            Ok(Some(token)) => {
                                callback((true, token));
                                return;
                            }
                            Ok(None) => {
                                thread::sleep(Duration::from_secs(state.interval.max(5) as u64))
                            }
                            Err(error) => {
                                callback((false, error.to_string()));
                                return;
                            }
                        }
                    }
                });
            }
            Err(error) => self.set_error(error.to_string()),
        }
    }

    pub fn cancel_oauth_login(&mut self) {
        self.oauth_generation.fetch_add(1, Ordering::SeqCst);
        self.oauth_user_code = QString::default();
        self.oauth_verification_uri = QString::default();
        self.oauthStateChanged();
    }

    pub fn copy_to_clipboard(&self, _text: QString) {}

    fn set_current_view(&mut self, value: &str) {
        if self.current_view.to_string() == value {
            return;
        }
        self.current_view = QString::from(value);
        self.currentViewChanged();
    }

    fn rebuild_selected_file_rows(&mut self) {
        let rows = if self.selected_file_index >= 0
            && (self.selected_file_index as usize) < self.file_diffs_store.len()
        {
            let file = &self.file_diffs_store[self.selected_file_index as usize];
            flatten_file_diff(file, self.selected_file_index as usize)
                .into_iter()
                .map(|row| self.flat_row_to_item(file, row))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        self.selected_file_rows_model.borrow_mut().reset_data(rows);
        self.selected_file_rows_changed();
    }

    fn flat_row_to_item(&self, file: &FileDiff, row: FlatDiffRow) -> DiffRowItem {
        match row.row_type {
            DiffRowType::FileHeader => DiffRowItem {
                row_type: QString::from("fileHeader"),
                file_index: row.file_index,
                hunk_index: row.hunk_index,
                line_index: row.line_index,
                old_line_number: -1,
                new_line_number: -1,
                text: QString::from(file.path.as_str()),
            },
            DiffRowType::HunkSeparator => DiffRowItem {
                row_type: QString::from("hunk"),
                file_index: row.file_index,
                hunk_index: row.hunk_index,
                line_index: row.line_index,
                old_line_number: -1,
                new_line_number: -1,
                text: QString::from(
                    file.hunks
                        .get(row.hunk_index.max(0) as usize)
                        .map(|hunk| hunk.header.as_str())
                        .unwrap_or(""),
                ),
            },
            _ => {
                let (old_line_number, new_line_number, text) =
                    row_text(file, &self.text_buffer_store, &row);
                DiffRowItem {
                    row_type: QString::from(match row.row_type {
                        DiffRowType::Context => "context",
                        DiffRowType::Added => "added",
                        DiffRowType::Removed => "removed",
                        DiffRowType::Modified => "modified",
                        DiffRowType::FileHeader => "fileHeader",
                        DiffRowType::HunkSeparator => "hunk",
                    }),
                    file_index: row.file_index,
                    hunk_index: row.hunk_index,
                    line_index: row.line_index,
                    old_line_number,
                    new_line_number,
                    text: QString::from(text.as_str()),
                }
            }
        }
    }

    fn add_recent_repository(&mut self, path: &str) {
        let mut repos = decode_string_list(&encode_qvariant_string_list(&self.recent_repositories));
        repos.retain(|item| item != path);
        repos.insert(0, path.to_owned());
        repos.truncate(10);
        self.recent_repositories = repos
            .iter()
            .cloned()
            .map(|s| QVariant::from(QString::from(s.as_str())))
            .collect();
        self.settings
            .set_string("recentRepositories", &encode_string_list(&repos));
        self.settings.sync();
        self.recent_repositories_changed();
    }

    fn set_error(&mut self, error: impl AsRef<str>) {
        self.error_message = QString::from(error.as_ref());
        self.error_message_changed();
    }

    fn clear_error(&mut self) {
        if self.error_message.to_string().is_empty() {
            return;
        }
        self.error_message = QString::default();
        self.error_message_changed();
    }

    fn abbreviate_ref(&self, value: &str) -> String {
        if !looks_like_oid(value) {
            return value.to_owned();
        }
        match self.git_service.resolve_oid_to_branch_name(value) {
            Ok(name) if !name.is_empty() => name,
            _ => value.chars().take(8).collect(),
        }
    }

    fn persist_settings(&mut self) {
        self.settings
            .set_string("repoPath", &self.repo_path.to_string());
        self.settings
            .set_string("leftRef", &self.left_ref.to_string());
        self.settings
            .set_string("rightRef", &self.right_ref.to_string());
        self.settings
            .set_string("compareMode", &self.compare_mode.to_string());
        self.settings
            .set_string("renderer", &self.renderer.to_string());
        self.settings
            .set_string("layoutMode", &self.layout_mode.to_string());
        self.settings.sync();
    }
}

fn row_text(file: &FileDiff, buffer: &TextBuffer, row: &FlatDiffRow) -> (i32, i32, String) {
    let Some(hunk) = file.hunks.get(row.hunk_index.max(0) as usize) else {
        return (-1, -1, String::new());
    };
    let main_line = hunk.lines.get(row.line_index.max(0) as usize);
    let old_line = hunk.lines.get(row.old_line_index.max(0) as usize);
    let new_line = hunk.lines.get(row.new_line_index.max(0) as usize);
    let old_num = old_line.and_then(|line| line.old_line_number).unwrap_or(-1);
    let new_num = new_line
        .and_then(|line| line.new_line_number)
        .or_else(|| main_line.and_then(|line| line.new_line_number))
        .unwrap_or(-1);
    let text = match row.row_type {
        DiffRowType::Modified => {
            let left = old_line
                .map(|line| line_text(line, buffer))
                .unwrap_or_default();
            let right = new_line
                .map(|line| line_text(line, buffer))
                .unwrap_or_default();
            if left.is_empty() {
                right
            } else if right.is_empty() {
                left
            } else {
                format!("{left} ⟶ {right}")
            }
        }
        _ => main_line
            .map(|line| line_text(line, buffer))
            .unwrap_or_default(),
    };
    (old_num, new_num, text)
}

fn line_text(line: &DiffLine, buffer: &TextBuffer) -> String {
    buffer.view(line.text_range).to_owned()
}

fn looks_like_oid(value: &str) -> bool {
    value.len() == 40 && value.chars().all(|c| c.is_ascii_hexdigit())
}

fn non_empty(value: String, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_owned()
    } else {
        value
    }
}

fn settings_file_path(name: &str) -> String {
    let base = std::env::var("DIFFY_SETTINGS_DIR")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| {
            std::env::var("XDG_CONFIG_HOME")
                .ok()
                .map(|v| format!("{v}/diffy"))
        })
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_owned());
            format!("{home}/.config/diffy")
        });
    let _ = std::fs::create_dir_all(&base);
    PathBuf::from(base).join(name).to_string_lossy().to_string()
}

fn decode_string_list(raw: &str) -> Vec<String> {
    serde_json::from_str(raw).unwrap_or_default()
}

fn encode_string_list(values: &[String]) -> String {
    serde_json::to_string(values).unwrap_or_else(|_| "[]".to_owned())
}

fn encode_qvariant_string_list(values: &QVariantList) -> String {
    let data = values
        .into_iter()
        .map(|item| item.to_qstring().to_string())
        .collect::<Vec<_>>();
    encode_string_list(&data)
}
