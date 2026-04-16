use std::cell::Cell;
use std::rc::Rc;

use halogen::view;

use crate::actions::Action;
use crate::core::vcs::git::{CommitInfo, StatusScope};
use crate::effects::Effect;
use crate::render::{Rect, RectPrimitive, RoundedRectPrimitive};
use crate::ui::components::{
    self, Button, ButtonSize, ButtonStyle, SegmentedControl, SegmentedItem,
};
use crate::ui::design::{Alpha, Ico, Rad, Sp, Sz};
use crate::ui::editor_element::{CursorSnapshot, text_editor_element};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::shell::CursorHint;
use crate::ui::state::{
    AppState, FocusTarget, SidebarMode, SidebarTab, SidebarWidthCache, WorkspaceSource,
};
use crate::ui::style::Styled;
use crate::ui::theme::{Color, Theme};

pub(crate) struct SidebarResizeDrag {
    origin_x: f32,
    starting_width: f32,
}

impl SidebarResizeDrag {
    pub fn new(origin_x: f32, starting_width: f32) -> Self {
        Self {
            origin_x,
            starting_width,
        }
    }
}

impl DragHandler for SidebarResizeDrag {
    fn on_move(&mut self, x: f32, _y: f32) -> Vec<Action> {
        let target_width = (self.starting_width + (x - self.origin_x)).round().max(0.0) as u32;
        vec![Action::SetSidebarWidthPx(target_width)]
    }

    fn on_release(&mut self, state: &crate::ui::state::AppState) -> DragReleaseResult {
        DragReleaseResult {
            actions: Vec::new(),
            effects: vec![Effect::SaveSettings(state.settings.clone())],
        }
    }

    fn cursor(&self) -> CursorHint {
        CursorHint::ResizeCol
    }
}

pub(crate) fn preferred_sidebar_width(
    state: &mut AppState,
    theme: &Theme,
    cx: &mut ElementContext,
    available_width: f32,
) -> f32 {
    let ui_scale = theme.metrics.ui_scale();
    let list_side_padding = Sp::MD * ui_scale;
    let row_side_padding = Sp::SM * 2.0 * ui_scale;
    let row_gap = Sp::SM * ui_scale;
    let stats_gap = Sp::XS * ui_scale;
    let header_side_padding = Sp::XXL + Sp::XS;
    let header_badge_outer_padding = Sp::SM * 2.0 * ui_scale;
    let header_badge_inner_padding = Sp::MD * ui_scale;
    let scrollbar_gutter = Ico::LG * ui_scale;
    let auto_min_width = theme.metrics.sidebar_width;
    let manual_min_width = (theme.metrics.sidebar_width * 0.64).round();
    let file_icon_width = Ico::XS;
    let hard_max = available_width.max(0.0);
    let max_width = if hard_max >= auto_min_width {
        (available_width - Sz::MAIN_SURFACE_MIN_W)
            .max(auto_min_width)
            .min(hard_max)
    } else {
        hard_max
    };
    let file_count = state.workspace.files.with(&state.store, |f| f.len());
    if file_count == 0 {
        return state
            .settings
            .sidebar_width_px
            .map(|width| width as f32)
            .unwrap_or(auto_min_width)
            .clamp(0.0, hard_max.max(0.0));
    }
    if max_width <= manual_min_width {
        return max_width;
    }
    if let Some(preferred_width) = state.settings.sidebar_width_px {
        return (preferred_width as f32).clamp(manual_min_width, max_width);
    }

    let compare_generation = state.workspace.compare_generation.get(&state.store);
    let cached_intrinsic_width =
        state
            .workspace
            .sidebar_auto_width
            .with(&state.store, |cache_opt| {
                cache_opt.and_then(|cache| {
                    (cache.compare_generation == compare_generation
                        && cache.ui_scale_pct == state.settings.ui_scale_pct)
                        .then_some(cache.intrinsic_width_px)
                })
            });

    let intrinsic_width = if let Some(width) = cached_intrinsic_width {
        width
    } else {
        let header_label_width = measure_text_width(
            cx.font_system,
            "FILES",
            theme.metrics.ui_small_font_size - 1.0,
            crate::render::FontKind::Ui,
            crate::render::FontWeight::Semibold,
        );
        let header_badge_width = if file_count == 0 {
            0.0
        } else {
            let count_width = measure_text_width(
                cx.font_system,
                &file_count.to_string(),
                theme.metrics.ui_small_font_size - 1.0,
                crate::render::FontKind::Ui,
                crate::render::FontWeight::Normal,
            );
            header_badge_outer_padding + header_badge_inner_padding + count_width
        };
        let header_width = header_side_padding + header_label_width + header_badge_width;

        let widest_row = state.workspace.files.with(&state.store, |files| {
            files
                .iter()
                .map(|file| {
                    let path_width = measure_text_width(
                        cx.font_system,
                        &file.path,
                        theme.metrics.ui_small_font_size,
                        crate::render::FontKind::Ui,
                        crate::render::FontWeight::Normal,
                    );

                    let stats_width = if file.additions > 0 || file.deletions > 0 {
                        let additions_width = measure_text_width(
                            cx.font_system,
                            &format!("+{}", file.additions),
                            theme.metrics.ui_small_font_size - 1.0,
                            crate::render::FontKind::Ui,
                            crate::render::FontWeight::Normal,
                        );
                        let deletions_width = measure_text_width(
                            cx.font_system,
                            &format!("\u{2212}{}", file.deletions),
                            theme.metrics.ui_small_font_size - 1.0,
                            crate::render::FontKind::Ui,
                            crate::render::FontWeight::Normal,
                        );
                        row_gap + additions_width + stats_gap + deletions_width
                    } else {
                        0.0
                    };

                    let status_badge_width = if !file.status.is_empty() {
                        row_gap + (theme.metrics.ui_small_font_size + Sp::XS).round()
                    } else {
                        0.0
                    };

                    list_side_padding
                        + row_side_padding
                        + file_icon_width
                        + row_gap
                        + path_width
                        + stats_width
                        + status_badge_width
                        + scrollbar_gutter
                })
                .fold(0.0_f32, f32::max)
        });

        let intrinsic_width = widest_row.max(header_width);
        state.workspace.sidebar_auto_width.set(
            &state.store,
            Some(SidebarWidthCache {
                compare_generation,
                ui_scale_pct: state.settings.ui_scale_pct,
                intrinsic_width_px: intrinsic_width,
            }),
        );
        intrinsic_width
    };

    intrinsic_width.clamp(auto_min_width, max_width)
}

pub(crate) fn sidebar_resizer(
    theme: &Theme,
    bounds_cell: Rc<Cell<Option<Rect>>>,
    starting_width: f32,
) -> Canvas {
    let tc = theme.colors;
    let scale = theme.metrics.ui_scale();
    let handle_width = (Sp::SM * scale).round().max(Sp::SM);
    let track_width = 1.0_f32;
    let thumb_width = (Sp::XXS * scale).round().max(2.0);
    let thumb_height = (Sp::XXL * scale).round().max(Sp::LG);

    canvas(move |bounds, scene, cx| {
        bounds_cell.set(Some(bounds));
        let hovered = cx
            .mouse_position
            .is_some_and(|(mx, my)| bounds.contains(mx, my));
        let center_x = bounds.x + bounds.width * 0.5;
        let center_y = bounds.y + bounds.height * 0.5;
        let line_color = if hovered {
            tc.border_variant
        } else {
            tc.border_variant.with_alpha(Alpha::SOFT)
        };
        let thumb_color = if hovered {
            tc.text_muted
        } else {
            tc.border_variant
        };

        let sw = starting_width;
        cx.push_click_handler(
            bounds,
            CursorHint::ResizeCol,
            ClickHandler::new(move |event| {
                ClickResult::CaptureDrag(Box::new(SidebarResizeDrag::new(event.x, sw)))
            }),
        );

        scene.rect(RectPrimitive {
            rect: Rect {
                x: center_x - track_width * 0.5,
                y: bounds.y,
                width: track_width,
                height: bounds.height,
            },
            color: line_color,
        });
        scene.rounded_rect(RoundedRectPrimitive::uniform(
            Rect {
                x: center_x - thumb_width * 0.5,
                y: center_y - thumb_height * 0.5,
                width: thumb_width,
                height: thumb_height,
            },
            thumb_width,
            thumb_color,
        ));
    })
    .w(handle_width)
}

pub(crate) fn sidebar(
    state: &AppState,
    theme: &Theme,
    sidebar_width: f32,
    _bounds_cell: Rc<Cell<Option<Rect>>>,
    cx: &ElementContext,
) -> AnyElement {
    let tc = &theme.colors;
    let all_files = state.workspace.files.get(&state.store);
    let file_count = all_files.len();
    let scale = theme.metrics.ui_scale();
    let filter = state.file_list.filter.get(&state.store);
    let has_filter = !filter.is_empty();
    let row_h = theme.metrics.ui_row_height.round();

    let range_commits = state.workspace.range_commits.get(&state.store);
    let workspace_source = state.workspace.source.get(&state.store);
    let show_tabs = workspace_source == WorkspaceSource::Compare && range_commits.len() > 1;
    let on_commits_tab = show_tabs && state.file_list.tab.get(&state.store) == SidebarTab::Commits;
    let is_drilled = state
        .workspace
        .pre_drill_compare
        .with(&state.store, |p| p.is_some());

    let tab_bar: Option<AnyElement> = if show_tabs {
        Some(view! { scale,
            <div class="flex-col" px={Sp::MD} pt={Sp::XS} pb={Sp::XS}>
                {SegmentedControl::new(vec![
                    SegmentedItem::new(
                        format!("Files {}", file_count),
                        Action::SetSidebarTab(SidebarTab::Files),
                        !on_commits_tab,
                    ),
                    SegmentedItem::new(
                        format!("Commits {}", range_commits.len()),
                        Action::SetSidebarTab(SidebarTab::Commits),
                        on_commits_tab,
                    ),
                ])}
            </div>
        })
    } else {
        None
    };

    if on_commits_tab {
        let filtered_commits: Vec<usize> = if has_filter {
            let haystack: Vec<String> = range_commits
                .iter()
                .map(|c| format!("{} {}", c.short_oid, c.summary))
                .collect();
            let haystack_refs: Vec<&str> = haystack.iter().map(|s| s.as_str()).collect();
            let config = neo_frizbee::Config {
                max_typos: Some(2),
                sort: false,
                ..Default::default()
            };
            let mut matches = neo_frizbee::match_list(&filter, &haystack_refs, &config);
            matches.sort_by(|a, b| b.score.cmp(&a.score));
            matches.iter().map(|m| m.index as usize).collect()
        } else {
            (0..range_commits.len()).collect()
        };

        let search_bar: Option<AnyElement> = if !range_commits.is_empty() {
            let search_focused = cx.is_focused(FocusTarget::SidebarSearch);
            let input = text_input("", &filter)
                .placeholder("Filter commits\u{2026}")
                .focused(search_focused)
                .focus_target(FocusTarget::SidebarSearch)
                .cursor(state.text_edit.cursor.get(&state.store))
                .anchor(state.text_edit.anchor.get(&state.store))
                .cursor_moved_at(state.text_edit.cursor_moved_at_ms.get(&state.store))
                .on_click(Action::SetFocus(Some(FocusTarget::SidebarSearch)))
                .bare()
                .w_full()
                .h(row_h);
            let hint = if !search_focused && !has_filter {
                Some("/")
            } else {
                None
            };
            Some(view! { scale,
                <div class="w-full" px={Sp::SM + Sp::XXS} pb={Sp::SM}>
                    {components::search_field(input, has_filter, Some(Action::ClearSidebarFilter), hint, theme)}
                </div>
            })
        } else {
            None
        };

        let content: AnyElement = if filtered_commits.is_empty() && has_filter {
            view! { scale,
                <div class="flex-1 items-center justify-center">
                    <div class="flex-col items-center" gap={Sp::SM}>
                        <icon svg={lucide::SEARCH} size={Ico::XL} color={tc.text_muted} />
                        <text class="text-sm" color={tc.text_muted}>{"No commits match filter."}</text>
                    </div>
                </div>
            }
        } else {
            let total_height = state.file_list_total_content_height(filtered_commits.len());
            let scroll_px = state.file_list.commits_scroll_offset_px.get(&state.store);
            let rows: Vec<AnyElement> = filtered_commits
                .iter()
                .map(|&i| commit_row(&range_commits[i], i, state, tc, scale, is_drilled))
                .collect();

            view! { scale,
                <div class="flex-1 flex-col" min_h={0.0}
                     px={Rad::LG} pt={Sp::XXS} gap={Sp::XS}
                     clip scroll_y={scroll_px}
                     scroll_total={total_height}
                     on_scroll={ScrollActionBuilder::Custom(Action::ScrollCommitListPx)}>
                    {...rows}
                </div>
            }
        };

        return view! { scale,
            <div class="flex-col shrink-0 h-full" min_h={0.0}
                 w={sidebar_width}
                 bg={tc.sidebar_background}
                 border_r={tc.border_variant}>
                {?tab_bar}
                {?search_bar}
                {content}
            </div>
        };
    }

    let is_tree = state.file_list.mode.get(&state.store) == SidebarMode::TreeView
        && workspace_source == WorkspaceSource::Compare;

    let filtered_indices: Vec<usize> = if has_filter {
        let haystack: Vec<&str> = all_files.iter().map(|f| f.path.as_str()).collect();
        let config = neo_frizbee::Config {
            max_typos: Some(2),
            sort: false,
            ..Default::default()
        };
        let mut matches = neo_frizbee::match_list(&filter, &haystack, &config);
        matches.sort_by(|a, b| b.score.cmp(&a.score));
        matches.iter().map(|m| m.index as usize).collect()
    } else {
        (0..file_count).collect()
    };
    let visible_count = filtered_indices.len();

    let total_adds: i32 = all_files.iter().map(|f| f.additions).sum();
    let total_dels: i32 = all_files.iter().map(|f| f.deletions).sum();

    let mode_icon = if is_tree {
        lucide::ROWS
    } else {
        lucide::FOLDER
    };
    let mode_tip = if is_tree { "List view" } else { "Tree view" };

    let header: Option<AnyElement> = if show_tabs {
        None
    } else {
        Some(view! { scale,
            <div class="flex-col" px={Sp::MD}>
                <div class="flex-row items-center" h={row_h} gap={Sp::SM}>
                    <text class="text-xs font-semibold" color={tc.text_muted}>{"FILES"}</text>
                    if file_count > 0 {
                        <div px={Rad::LG} py={Sp::XXS} rounded={Rad::LG}
                             bg={Color::rgba(255, 255, 255, 10)}>
                            <text class="text-xs" color={tc.text_muted}>{file_count.to_string()}</text>
                        </div>
                    }
                    <spacer />
                    if file_count > 0 && workspace_source == WorkspaceSource::Compare {
                        <Button action={Action::ToggleSidebarMode}
                                tooltip={mode_tip}
                                fixed_size={Sz::MODE_TOGGLE}>
                            <Icon>{mode_icon}</Icon>
                        </Button>
                    }
                </div>
                if file_count > 0 {
                    <div class="flex-row items-center" h={row_h} gap={Sp::XS}>
                        {components::stat_summary(
                            file_count,
                            total_adds.unsigned_abs(),
                            total_dels.unsigned_abs(),
                        ).compact()}
                    </div>
                }
            </div>
        })
    };

    let files_header: Option<AnyElement> = if show_tabs {
        Some(view! { scale,
            <div class="flex-col" px={Sp::MD}>
                <div class="flex-row items-center" h={row_h} gap={Sp::SM}>
                    if file_count > 0 {
                        {components::stat_summary(
                            file_count,
                            total_adds.unsigned_abs(),
                            total_dels.unsigned_abs(),
                        ).compact()}
                    }
                    <spacer />
                    if file_count > 0 && workspace_source == WorkspaceSource::Compare {
                        <Button action={Action::ToggleSidebarMode}
                                tooltip={mode_tip}
                                fixed_size={Sz::MODE_TOGGLE}>
                            <Icon>{mode_icon}</Icon>
                        </Button>
                    }
                </div>
            </div>
        })
    } else {
        None
    };

    let search_bar: Option<AnyElement> = if file_count > 0 {
        let search_focused = cx.is_focused(FocusTarget::SidebarSearch);
        let input = text_input("", &filter)
            .placeholder("Filter files\u{2026}")
            .focused(search_focused)
            .focus_target(FocusTarget::SidebarSearch)
            .cursor(state.text_edit.cursor.get(&state.store))
            .anchor(state.text_edit.anchor.get(&state.store))
            .cursor_moved_at(state.text_edit.cursor_moved_at_ms.get(&state.store))
            .on_click(Action::SetFocus(Some(FocusTarget::SidebarSearch)))
            .bare()
            .w_full()
            .h(row_h);
        let hint = if !search_focused && !has_filter {
            Some("/")
        } else {
            None
        };
        Some(view! { scale,
            <div class="w-full" px={Sp::SM + Sp::XXS} pb={Sp::SM}>
                {components::search_field(input, has_filter, Some(Action::ClearSidebarFilter), hint, theme)}
            </div>
        })
    } else {
        None
    };

    let content: Option<AnyElement> = if all_files.is_empty() {
        let has_repo = state.compare.repo_path.with(&state.store, |p| p.is_some());
        let (icon, msg) = if has_repo {
            if workspace_source == WorkspaceSource::Status {
                (lucide::CHECK, "Working tree clean.")
            } else {
                (lucide::GIT_COMPARE, "Run a compare to see changes.")
            }
        } else {
            (lucide::FOLDER_OPEN, "Open a repository to start.")
        };
        Some(view! { scale,
            <div class="flex-1 items-center justify-center" pb={row_h}>
                <div class="flex-col items-center" gap={Sp::SM}>
                    <icon svg={icon} size={Ico::XL} color={tc.text_muted} />
                    <text class="text-sm" color={tc.text_muted}>{msg}</text>
                </div>
            </div>
        })
    } else if visible_count == 0 && has_filter {
        Some(view! { scale,
            <div class="flex-1 items-center justify-center">
                <div class="flex-col items-center" gap={Sp::SM}>
                    <icon svg={lucide::SEARCH} size={Ico::XL} color={tc.text_muted} />
                    <text class="text-sm" color={tc.text_muted}>{"No files match filter."}</text>
                </div>
            </div>
        })
    } else if is_tree && !has_filter {
        let entries: Vec<components::FileTreeEntry> =
            state.workspace.status_items.with(&state.store, |items| {
                filtered_indices
                    .iter()
                    .map(|&i| {
                        let f = &all_files[i];
                        components::FileTreeEntry {
                            path: f.path.clone(),
                            status: f.status.clone(),
                            scope: items
                                .get(i)
                                .filter(|_| workspace_source == WorkspaceSource::Status)
                                .map(|item| item.scope.label().to_owned()),
                            additions: f.additions,
                            deletions: f.deletions,
                        }
                    })
                    .collect()
            });

        let expanded_folders = state.file_list.expanded_folders.get(&state.store);
        let expanded_count = expanded_folders.len();
        let tree = components::file_tree(entries)
            .expanded(expanded_folders)
            .selected(state.workspace.selected_file_index.get(&state.store))
            .on_select_file(Action::SelectFile)
            .on_toggle_folder(Action::ToggleFolder);

        let row_count = visible_count + expanded_count;
        let row_height = state.file_list.row_height.get(&state.store);
        let total_height = row_count as f32 * (row_height + state.file_list.gap.get(&state.store));
        let scroll_px = state.file_list.scroll_offset_px.get(&state.store);

        Some(view! { scale,
            <div class="flex-1 flex-col" min_h={0.0}
                 clip scroll_y={scroll_px}
                 scroll_total={total_height}
                 on_scroll={ScrollActionBuilder::FileList}>
                {tree}
            </div>
        })
    } else {
        let grouped_status = workspace_source == WorkspaceSource::Status && !has_filter;
        let total_height = state.file_list_total_content_height(if grouped_status {
            state.sidebar_row_count()
        } else {
            visible_count
        });
        let scroll_px = state.file_list.scroll_offset_px.get(&state.store);

        let rows: Vec<AnyElement> = if grouped_status {
            let scopes_by_index: Vec<Option<StatusScope>> =
                state.workspace.status_items.with(&state.store, |items| {
                    filtered_indices
                        .iter()
                        .map(|&index| items.get(index).map(|item| item.scope))
                        .collect()
                });
            let mut rows = Vec::new();
            let mut last_scope = None;
            for (pos, &index) in filtered_indices.iter().enumerate() {
                let scope = scopes_by_index[pos];
                if scope != last_scope {
                    if let Some(scope) = scope {
                        rows.push(status_section_row(scope, tc, scale, row_h));
                    }
                    last_scope = scope;
                }
                rows.push(file_row(&all_files[index], index, state, tc, scale));
            }
            rows
        } else {
            filtered_indices
                .iter()
                .map(|&index| file_row(&all_files[index], index, state, tc, scale))
                .collect()
        };

        Some(view! { scale,
            <div class="flex-1 flex-col" min_h={0.0}
                 px={Rad::LG} pt={Sp::XXS} gap={Sp::XS}
                 clip scroll_y={scroll_px}
                 scroll_total={total_height}
                 on_scroll={ScrollActionBuilder::FileList}>
                {...rows}
            </div>
        })
    };

    let commit_box: Option<AnyElement> = if workspace_source == WorkspaceSource::Status {
        let commit_focused = cx.is_focused(FocusTarget::CommitEditor);
        let has_staged = state.workspace.status_items.with(&state.store, |items| {
            items.iter().any(|item| item.scope == StatusScope::Staged)
        });
        let can_commit = has_staged && !state.commit_editor.text().trim().is_empty();
        let box_h = (Sz::COMMIT_BOX_H * scale).round();
        let cursor_snap = CursorSnapshot {
            x: state.commit_editor.cursor_pos.x,
            y: state.commit_editor.cursor_pos.y,
            moved_at_ms: state.commit_editor.cursor_moved_at_ms,
        };
        let sel_rects = state.commit_editor.selection_rects();
        let editor_el = text_editor_element()
            .placeholder("Enter commit message")
            .is_empty(state.commit_editor.is_empty())
            .focused(commit_focused)
            .focus_target(FocusTarget::CommitEditor)
            .font_size(theme.metrics.ui_small_font_size)
            .text_color(tc.text)
            .cursor(cursor_snap)
            .selection(sel_rects)
            .content_height(state.commit_editor.content_height())
            .scroll_y(state.commit_editor.scroll_y)
            .w_full()
            .flex_1();
        Some(view! { scale,
            <div class="flex-col shrink-0" px={Sp::SM + Sp::XXS} py={Sp::SM}>
                <div class="flex-col w-full"
                     h={box_h}
                     rounded={Rad::LG}
                     border={tc.border_variant}
                     @when { commit_focused } { border={tc.accent} }>
                    <div class="flex-1 w-full" min_h={0.0}
                         px={Sp::SM} pt={Sp::XS}>
                        {editor_el}
                    </div>
                    <div class="flex-row items-center" px={Sp::SM} pb={Sp::SM}>
                        <spacer />
                        {Button::new(Action::SubmitCommit)
                            .label("Commit")
                            .style(ButtonStyle::Subtle)
                            .disabled(!can_commit)}
                    </div>
                </div>
            </div>
        })
    } else {
        None
    };

    view! { scale,
        <div class="flex-col shrink-0 h-full" min_h={0.0}
             w={sidebar_width}
             bg={tc.sidebar_background}
             border_r={tc.border_variant}>
            {?tab_bar}
            {?header}
            {?files_header}
            {?search_bar}
            {?content}
            {?commit_box}
        </div>
    }
}

fn status_section_row(
    scope: StatusScope,
    tc: &crate::ui::theme::ThemeColors,
    scale: f32,
    row_height: f32,
) -> AnyElement {
    let label = scope.label();
    let section_action: Option<(Action, &str, &str)> = match scope {
        StatusScope::Unstaged | StatusScope::Untracked => {
            Some((Action::StageAllFiles, lucide::PLUS, "Stage All"))
        }
        StatusScope::Staged => Some((Action::UnstageAllFiles, lucide::MINUS, "Unstage All")),
    };

    view! { scale,
        <div class="w-full shrink-0 flex-row items-center"
             h={row_height}
             px={Sp::SM}>
            <text class="text-xs font-semibold" color={tc.text_muted}>{label}</text>
            <spacer />
            if let Some((action, icon, btn_label)) = section_action {
                <Button action={action}
                        tooltip={btn_label}
                        style={ButtonStyle::Subtle}
                        size={ButtonSize::Compact}>
                    <Icon>{icon}</Icon>
                    <Label>{btn_label}</Label>
                </Button>
            }
        </div>
    }
    .into_any()
}

fn file_row(
    file: &crate::ui::state::FileListEntry,
    index: usize,
    state: &AppState,
    tc: &crate::ui::theme::ThemeColors,
    scale: f32,
) -> AnyElement {
    let selected = state.workspace.selected_file_index.get(&state.store) == Some(index);
    let viewed = state
        .file_list
        .viewed_files
        .with(&state.store, |s| s.contains(&index));
    let text_color = if selected { tc.text_strong } else { tc.text };
    let row_height = state.file_list.row_height.get(&state.store);

    let (filename, dir_path) = match file.path.rfind('/') {
        Some(pos) => (&file.path[pos + 1..], Some(&file.path[..pos])),
        None => (file.path.as_str(), None),
    };

    let dir_el: Option<AnyElement> =
        dir_path.map(|p| text(p).text_xs().color(tc.text_muted).truncate().into_any());

    let has_stats = file.additions > 0 || file.deletions > 0;
    let has_status = !file.status.is_empty();
    let is_status_view = state.workspace.source.get(&state.store) == WorkspaceSource::Status;
    let status_scope = state
        .workspace
        .status_items
        .with(&state.store, |items| items.get(index).cloned())
        .filter(|_| is_status_view && !state.file_list.filter.with(&state.store, |s| s.is_empty()))
        .map(|item| item.scope.label());

    let stage_action: Option<(Action, &str, &str)> = state
        .workspace
        .status_items
        .with(&state.store, |items| items.get(index).map(|i| i.scope))
        .filter(|_| is_status_view)
        .and_then(|scope| match scope {
            StatusScope::Unstaged | StatusScope::Untracked => {
                Some((Action::StageFile(index), lucide::PLUS, "Stage"))
            }
            StatusScope::Staged => Some((Action::UnstageFile(index), lucide::MINUS, "Unstage")),
        });

    let hovered = state.file_list.hovered_index.get(&state.store) == Some(index);
    let show_stage_btn = hovered || selected;
    let stage_btn: Option<AnyElement> =
        stage_action
            .filter(|_| show_stage_btn)
            .map(|(action, icon, tooltip)| {
                view! { scale,
                    <Button action={action}
                            tooltip={tooltip}
                            fixed_size={Sz::MODE_TOGGLE}>
                        <Icon>{icon}</Icon>
                    </Button>
                }
            });

    view! { scale,
        <div class="w-full shrink-0 flex-row items-center"
             h={row_height} px={Sp::SM} gap={Sp::SM}
             on_click={Action::SelectFile(index)}
             hit_identity={HitIdentity::File(index)}
             cursor={CursorHint::Pointer}
             @when { selected } { bg={tc.sidebar_row_selected} border_l={tc.accent} }
             @when { !selected } { hover_bg={tc.sidebar_row_hover} }>
            {components::file_icon(&file.path, Ico::LG).selected(selected)}
            <div class="flex-1 flex-row items-center overflow-hidden" min_w={0.0} gap={Sp::SM}>
                <div class="shrink-0">
                    <text class="text-sm" color={text_color}>{filename}</text>
                </div>
                {?dir_el}
            </div>
            if has_stats {
                <div class="flex-row shrink-0" gap={Sp::XS}>
                    <text class="text-xs" color={tc.line_add_text}>{format!("+{}", file.additions)}</text>
                    <text class="text-xs" color={tc.line_del_text}>{format!("\u{2212}{}", file.deletions)}</text>
                </div>
            }
            if let Some(scope) = status_scope {
                <div class="shrink-0">
                    <text class="text-xs" color={tc.text_muted}>{scope}</text>
                </div>
            }
            {?stage_btn}
            if has_status {
                {components::status_badge(&file.status)}
            }
            if viewed {
                <icon svg={lucide::CHECK} size={Ico::XS} color={tc.line_add_text} />
            }
        </div>
    }
}

fn commit_row(
    commit: &CommitInfo,
    _index: usize,
    state: &AppState,
    tc: &crate::ui::theme::ThemeColors,
    scale: f32,
    is_drilled: bool,
) -> AnyElement {
    let row_height = state.file_list.row_height.get(&state.store);
    let selected = is_drilled
        && state
            .compare
            .left_ref
            .with(&state.store, |left| commit.oid.starts_with(left));
    let action = if selected {
        Action::ClearSidebarCommit
    } else {
        Action::SelectSidebarCommit(commit.oid.clone())
    };

    view! { scale,
        <div class="w-full shrink-0 flex-row items-center"
             h={row_height} px={Sp::SM} gap={Sp::SM}
             on_click={action}
             cursor={CursorHint::Pointer}
             @when { selected } { bg={tc.sidebar_row_selected} border_l={tc.accent} }
             @when { !selected } { hover_bg={tc.sidebar_row_hover} }>
            <icon svg={lucide::CIRCLE_DOT} size={Ico::SM} color={if selected { tc.accent } else { tc.text_muted }} />
            <div class="flex-1 overflow-hidden" min_w={0.0}>
                <text class="text-sm" color={if selected { tc.text_strong } else { tc.text }}>{&commit.summary}</text>
            </div>
            <div class="shrink-0">
                <text class="text-xs font-mono" color={tc.text_muted}>{&commit.short_oid}</text>
            </div>
        </div>
    }
}
