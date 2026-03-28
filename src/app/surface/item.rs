use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex;
use qmetaobject::prelude::*;
use qmetaobject::scenegraph::{ContainerNode, RectangleNode, SGNode};
use qmetaobject::{QMouseEvent, QMouseEventType, QQuickItem, QVariantMap};
use rayon::ThreadPool;
use rayon::ThreadPoolBuilder;

use crate::app::theme::default_mono_family;
use crate::core::rendering::{DiffRowType, FlatDiffRow, PreparedRow};
use crate::core::text::buffer::{TextBuffer, TextRange};
use crate::core::text::token::TokenRange;

cpp! {{
    #include <QtCore/QAbstractItemModel>
    #include <QtCore/QByteArray>
    #include <QtCore/QHash>
    #include <QtCore/QMetaObject>
    #include <QtCore/QModelIndex>
    #include <QtQuick/QQuickItem>
    #include <QtQuick/QQuickWindow>
    #include <QtQuick/QSGNode>
    #include <QtCore/QVariant>
}}

#[repr(C)]
enum QQuickItemFlag {
    ItemHasContents = 0x08,
    ItemAcceptsInputMethod = 0x02,
    ItemIsFocusScope = 0x04,
}

#[derive(Debug, Clone, Copy, Default)]
struct PerfBucket {
    sum: f64,
    peak: f64,
    count: i32,
}

impl PerfBucket {
    fn record(&mut self, ms: f64) {
        self.sum += ms;
        self.peak = self.peak.max(ms);
        self.count += 1;
    }

    fn avg(self) -> f64 {
        if self.count > 0 {
            self.sum / f64::from(self.count)
        } else {
            0.0
        }
    }
}

#[derive(Debug, Clone)]
struct PerfSession {
    paint: PerfBucket,
    raster: PerfBucket,
    _upload: PerfBucket,
    rebuild: PerfBucket,
    display_rebuild: PerfBucket,
    metrics: PerfBucket,
    total_frames: i32,
    dropped_frames: i32,
    total_cache_hits: i32,
    total_cache_misses: i32,
    start_time: Instant,
}

impl Default for PerfSession {
    fn default() -> Self {
        Self {
            paint: PerfBucket::default(),
            raster: PerfBucket::default(),
            _upload: PerfBucket::default(),
            rebuild: PerfBucket::default(),
            display_rebuild: PerfBucket::default(),
            metrics: PerfBucket::default(),
            total_frames: 0,
            dropped_frames: 0,
            total_cache_hits: 0,
            total_cache_misses: 0,
            start_time: Instant::now(),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct TileEntry {
    _key: u64,
    _row_index: i32,
    _last_used_tick: u64,
}

#[derive(Debug, Default)]
struct TileCacheState {
    image_cache: HashMap<u64, TileEntry>,
    pending_keys: HashMap<u64, i32>,
    ready_keys: HashMap<u64, i32>,
}

#[derive(Debug, Clone, Default)]
struct SurfaceRow {
    flat: FlatDiffRow,
    text_range: TextRange,
    syntax_tokens: TokenRange,
    change_tokens: TokenRange,
    measured_width: f64,
    y: f64,
    height: f64,
}

impl SurfaceRow {
    fn display_text<'a>(&self, text_buffer: &'a TextBuffer) -> &'a str {
        text_buffer.view(self.text_range)
    }
}

fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

fn qvariant_to_object_ptr(variant: &QVariant) -> *mut std::ffi::c_void {
    let variant = variant as *const QVariant;
    cpp!(unsafe [variant as "const QVariant*"] -> *mut std::ffi::c_void as "QObject*" {
        return variant->value<QObject*>();
    })
}

fn model_row_count(model: *mut std::ffi::c_void) -> i32 {
    cpp!(unsafe [model as "QObject*"] -> i32 as "int" {
        auto *itemModel = qobject_cast<QAbstractItemModel*>(model);
        return itemModel ? itemModel->rowCount() : 0;
    })
}

fn model_role(model: *mut std::ffi::c_void, name: &QByteArray) -> i32 {
    let name = name as *const QByteArray;
    cpp!(unsafe [model as "QObject*", name as "const QByteArray*"] -> i32 as "int" {
        auto *itemModel = qobject_cast<QAbstractItemModel*>(model);
        if (!itemModel) return -1;
        const auto names = itemModel->roleNames();
        for (auto it = names.constBegin(); it != names.constEnd(); ++it) {
            if (it.value() == *name) return it.key();
        }
        return -1;
    })
}

fn model_data(model: *mut std::ffi::c_void, row: i32, role: i32) -> QVariant {
    cpp!(unsafe [model as "QObject*", row as "int", role as "int"] -> QVariant as "QVariant" {
        auto *itemModel = qobject_cast<QAbstractItemModel*>(model);
        if (!itemModel || role < 0) return QVariant();
        return itemModel->data(itemModel->index(row, 0), role);
    })
}

fn qvariant_to_i32(value: &QVariant) -> i32 {
    value.to_qstring().to_int(10).unwrap_or_default()
}

fn qvariant_to_string(value: &QVariant) -> String {
    value.to_qstring().to_string()
}

fn row_type_from_string(value: &str) -> DiffRowType {
    match value {
        "file-header" => DiffRowType::FileHeader,
        "hunk" | "hunk-separator" => DiffRowType::HunkSeparator,
        "added" | "add" => DiffRowType::Added,
        "removed" | "del" | "deleted" => DiffRowType::Removed,
        "modified" | "change" => DiffRowType::Modified,
        _ => DiffRowType::Context,
    }
}

fn row_height_for(
    row_type: DiffRowType,
    file_header_height: f64,
    hunk_height: f64,
    row_height: f64,
) -> f64 {
    match row_type {
        DiffRowType::FileHeader => file_header_height,
        DiffRowType::HunkSeparator => hunk_height,
        _ => row_height,
    }
}

fn background_for_row(row: &SurfaceRow) -> QColor {
    match row.flat.row_type {
        DiffRowType::FileHeader => QColor::from_name("#2b2f36"),
        DiffRowType::HunkSeparator => QColor::from_name("#353b45"),
        DiffRowType::Added => QColor::from_name("#1f2d24"),
        DiffRowType::Removed => QColor::from_name("#2d2024"),
        DiffRowType::Modified => QColor::from_name("#2a2836"),
        DiffRowType::Context => QColor::from_name("#282c33"),
    }
}

fn overlay_for_row(selected: bool, hovered: bool) -> QColor {
    if selected {
        let mut color = QColor::from_name("#7c6f64");
        color.set_alpha(110);
        color
    } else if hovered {
        let mut color = QColor::from_name("#504945");
        color.set_alpha(80);
        color
    } else {
        QColor::default()
    }
}

#[allow(non_snake_case)]
#[derive(QObject)]
pub struct DiffSurfaceItem {
    base: qt_base_class!(trait QQuickItem),

    rows_model: qt_property!(QVariant; WRITE set_rows_model NOTIFY rows_model_changed ALIAS rowsModel),
    rows_model_changed: qt_signal!(),

    layout_mode: qt_property!(QString; WRITE set_layout_mode NOTIFY layout_mode_changed ALIAS layoutMode),
    layout_mode_changed: qt_signal!(),
    compare_generation: qt_property!(i32; WRITE set_compare_generation NOTIFY compare_generation_changed ALIAS compareGeneration),
    compare_generation_changed: qt_signal!(),

    file_path: qt_property!(QString; WRITE set_file_path NOTIFY file_path_changed ALIAS filePath),
    file_path_changed: qt_signal!(),
    file_status: qt_property!(QString; WRITE set_file_status NOTIFY file_status_changed ALIAS fileStatus),
    file_status_changed: qt_signal!(),
    additions: qt_property!(i32; WRITE set_additions NOTIFY additions_changed ALIAS additions),
    additions_changed: qt_signal!(),
    deletions: qt_property!(i32; WRITE set_deletions NOTIFY deletions_changed ALIAS deletions),
    deletions_changed: qt_signal!(),

    palette: qt_property!(QVariantMap; WRITE set_palette NOTIFY palette_changed ALIAS palette),
    palette_changed: qt_signal!(),
    monoFontFamily: qt_property!(QString; WRITE set_mono_font_family NOTIFY mono_font_family_changed ALIAS monoFontFamily),
    mono_font_family_changed: qt_signal!(),

    content_height: qt_property!(f64; READ get_content_height NOTIFY content_height_changed ALIAS contentHeight),
    content_height_changed: qt_signal!(),
    content_width: qt_property!(f64; READ get_content_width NOTIFY content_width_changed ALIAS contentWidth),
    content_width_changed: qt_signal!(),
    viewport_x: qt_property!(f64; WRITE set_viewport_x NOTIFY viewport_x_changed ALIAS viewportX),
    viewport_x_changed: qt_signal!(),
    viewport_y: qt_property!(f64; WRITE set_viewport_y NOTIFY viewport_y_changed ALIAS viewportY),
    viewport_y_changed: qt_signal!(),
    leftViewportX: qt_property!(f64; WRITE set_left_viewport_x NOTIFY left_viewport_x_changed ALIAS leftViewportX),
    left_viewport_x_changed: qt_signal!(),
    rightViewportX: qt_property!(f64; WRITE set_right_viewport_x NOTIFY right_viewport_x_changed ALIAS rightViewportX),
    right_viewport_x_changed: qt_signal!(),
    viewport_height: qt_property!(f64; WRITE set_viewport_height NOTIFY viewport_height_changed ALIAS viewportHeight),
    viewport_height_changed: qt_signal!(),

    wrap_enabled: qt_property!(bool; WRITE set_wrap_enabled NOTIFY wrap_enabled_changed ALIAS wrapEnabled),
    wrap_enabled_changed: qt_signal!(),
    wrap_column: qt_property!(i32; WRITE set_wrap_column NOTIFY wrap_column_changed ALIAS wrapColumn),
    wrap_column_changed: qt_signal!(),

    paint_count: qt_property!(i32; READ get_paint_count NOTIFY paintCountChanged ALIAS paintCount),
    paintCountChanged: qt_signal!(),
    display_row_count: qt_property!(i32; READ get_display_row_count NOTIFY display_row_count_changed ALIAS displayRowCount),
    display_row_count_changed: qt_signal!(),
    tileCacheHits: qt_property!(i32; READ get_tile_cache_hits NOTIFY tile_stats_changed ALIAS tileCacheHits),
    tileCacheMisses: qt_property!(i32; READ get_tile_cache_misses NOTIFY tile_stats_changed ALIAS tileCacheMisses),
    textureUploadCount: qt_property!(i32; READ get_texture_upload_count NOTIFY tile_stats_changed ALIAS textureUploadCount),
    residentTileCount: qt_property!(i32; READ get_resident_tile_count NOTIFY tile_stats_changed ALIAS residentTileCount),
    pendingTileJobCount: qt_property!(i32; READ get_pending_tile_job_count NOTIFY tile_stats_changed ALIAS pendingTileJobCount),
    tile_stats_changed: qt_signal!(),
    lastPaintTimeMs: qt_property!(f64; READ get_last_paint_time_ms NOTIFY perfStatsChanged ALIAS lastPaintTimeMs),
    lastRasterTimeMs: qt_property!(f64; READ get_last_raster_time_ms NOTIFY perfStatsChanged ALIAS lastRasterTimeMs),
    lastTextureUploadTimeMs: qt_property!(f64; READ get_last_texture_upload_time_ms NOTIFY perfStatsChanged ALIAS lastTextureUploadTimeMs),
    lastRowsRebuildTimeMs: qt_property!(f64; READ get_last_rows_rebuild_time_ms NOTIFY perfStatsChanged ALIAS lastRowsRebuildTimeMs),
    lastDisplayRowsRebuildTimeMs: qt_property!(f64; READ get_last_display_rows_rebuild_time_ms NOTIFY perfStatsChanged ALIAS lastDisplayRowsRebuildTimeMs),
    lastMetricsRecalcTimeMs: qt_property!(f64; READ get_last_metrics_recalc_time_ms NOTIFY perfStatsChanged ALIAS lastMetricsRecalcTimeMs),
    perfStatsChanged: qt_signal!(),

    scrollToYRequested: qt_signal!(value: f64),
    nextFileRequested: qt_signal!(),
    previousFileRequested: qt_signal!(),

    reset_perf_stats: qt_method!(fn(&mut self)),
    dump_perf_report: qt_method!(fn(&self)),

    rows: Vec<SurfaceRow>,
    text_buffer: TextBuffer,
    char_width: f64,
    row_height_value: f64,
    file_header_height_value: f64,
    hunk_height_value: f64,
    line_number_digits: i32,
    max_text_width: f64,
    first_visible_row: i32,
    last_visible_row: i32,
    hovered_row: i32,
    selection_anchor_row: i32,
    selection_cursor_row: i32,
    content_generation: u64,
    tile_use_tick: u64,
    tile_state: Arc<Mutex<TileCacheState>>,
    raster_pool: Arc<ThreadPool>,
    perf_session: PerfSession,
}

impl Default for DiffSurfaceItem {
    fn default() -> Self {
        Self {
            base: Default::default(),
            rows_model: QVariant::default(),
            rows_model_changed: Default::default(),
            layout_mode: QString::from("unified"),
            layout_mode_changed: Default::default(),
            compare_generation: 0,
            compare_generation_changed: Default::default(),
            file_path: QString::default(),
            file_path_changed: Default::default(),
            file_status: QString::from("M"),
            file_status_changed: Default::default(),
            additions: 0,
            additions_changed: Default::default(),
            deletions: 0,
            deletions_changed: Default::default(),
            palette: QVariantMap::default(),
            palette_changed: Default::default(),
            monoFontFamily: QString::from(default_mono_family()),
            mono_font_family_changed: Default::default(),
            content_height: 0.0,
            content_height_changed: Default::default(),
            content_width: 0.0,
            content_width_changed: Default::default(),
            viewport_x: 0.0,
            viewport_x_changed: Default::default(),
            viewport_y: 0.0,
            viewport_y_changed: Default::default(),
            leftViewportX: 0.0,
            left_viewport_x_changed: Default::default(),
            rightViewportX: 0.0,
            right_viewport_x_changed: Default::default(),
            viewport_height: 0.0,
            viewport_height_changed: Default::default(),
            wrap_enabled: false,
            wrap_enabled_changed: Default::default(),
            wrap_column: 0,
            wrap_column_changed: Default::default(),
            paint_count: 0,
            paintCountChanged: Default::default(),
            display_row_count: 0,
            display_row_count_changed: Default::default(),
            tileCacheHits: 0,
            tileCacheMisses: 0,
            textureUploadCount: 0,
            residentTileCount: 0,
            pendingTileJobCount: 0,
            tile_stats_changed: Default::default(),
            lastPaintTimeMs: 0.0,
            lastRasterTimeMs: 0.0,
            lastTextureUploadTimeMs: 0.0,
            lastRowsRebuildTimeMs: 0.0,
            lastDisplayRowsRebuildTimeMs: 0.0,
            lastMetricsRecalcTimeMs: 0.0,
            perfStatsChanged: Default::default(),
            scrollToYRequested: Default::default(),
            nextFileRequested: Default::default(),
            previousFileRequested: Default::default(),
            reset_perf_stats: Default::default(),
            dump_perf_report: Default::default(),
            rows: Vec::new(),
            text_buffer: TextBuffer::default(),
            char_width: 8.0,
            row_height_value: 20.0,
            file_header_height_value: 32.0,
            hunk_height_value: 28.0,
            line_number_digits: 3,
            max_text_width: 0.0,
            first_visible_row: -1,
            last_visible_row: -1,
            hovered_row: -1,
            selection_anchor_row: -1,
            selection_cursor_row: -1,
            content_generation: 1,
            tile_use_tick: 0,
            tile_state: Arc::new(Mutex::new(TileCacheState::default())),
            raster_pool: Arc::new(
                ThreadPoolBuilder::new()
                    .thread_name(|idx| format!("diff-surface-raster-{idx}"))
                    .num_threads(
                        std::thread::available_parallelism()
                            .map_or(1, |v| v.get().saturating_sub(1).max(1)),
                    )
                    .build()
                    .expect("rayon thread pool"),
            ),
            perf_session: PerfSession::default(),
        }
    }
}

impl DiffSurfaceItem {
    fn cpp_item_ptr(&self) -> *mut std::ffi::c_void {
        self.get_cpp_object()
    }

    fn set_flag(&mut self, flag: QQuickItemFlag) {
        let obj = self.cpp_item_ptr();
        cpp!(unsafe [obj as "QQuickItem*", flag as "QQuickItem::Flag"] {
            if (obj) obj->setFlag(flag, true);
        });
    }

    fn set_accept_hover_events(&mut self, enabled: bool) {
        let obj = self.cpp_item_ptr();
        cpp!(unsafe [obj as "QQuickItem*", enabled as "bool"] {
            if (obj) obj->setAcceptHoverEvents(enabled);
        });
    }

    fn set_accepted_mouse_buttons(&mut self) {
        let obj = self.cpp_item_ptr();
        cpp!(unsafe [obj as "QQuickItem*"] {
            if (obj) obj->setAcceptedMouseButtons(Qt::LeftButton);
        });
    }

    fn force_focus(&mut self) {
        let obj = self.cpp_item_ptr();
        cpp!(unsafe [obj as "QQuickItem*"] {
            if (obj) obj->forceActiveFocus(Qt::MouseFocusReason);
        });
    }

    fn update_item(&self) {
        (self as &dyn QQuickItem).update();
    }

    fn bounding_width(&self) -> f64 {
        (self as &dyn QQuickItem).bounding_rect().width
    }

    fn bounding_height(&self) -> f64 {
        (self as &dyn QQuickItem).bounding_rect().height
    }

    pub fn get_content_height(&self) -> f64 {
        self.content_height
    }

    pub fn get_content_width(&self) -> f64 {
        self.content_width
    }

    pub fn get_paint_count(&self) -> i32 {
        self.paint_count
    }

    pub fn get_display_row_count(&self) -> i32 {
        self.display_row_count
    }

    pub fn get_tile_cache_hits(&self) -> i32 {
        self.tileCacheHits
    }

    pub fn get_tile_cache_misses(&self) -> i32 {
        self.tileCacheMisses
    }

    pub fn get_texture_upload_count(&self) -> i32 {
        self.textureUploadCount
    }

    pub fn get_resident_tile_count(&self) -> i32 {
        self.residentTileCount
    }

    pub fn get_pending_tile_job_count(&self) -> i32 {
        self.pendingTileJobCount
    }

    pub fn get_last_paint_time_ms(&self) -> f64 {
        self.lastPaintTimeMs
    }

    pub fn get_last_raster_time_ms(&self) -> f64 {
        self.lastRasterTimeMs
    }

    pub fn get_last_texture_upload_time_ms(&self) -> f64 {
        self.lastTextureUploadTimeMs
    }

    pub fn get_last_rows_rebuild_time_ms(&self) -> f64 {
        self.lastRowsRebuildTimeMs
    }

    pub fn get_last_display_rows_rebuild_time_ms(&self) -> f64 {
        self.lastDisplayRowsRebuildTimeMs
    }

    pub fn get_last_metrics_recalc_time_ms(&self) -> f64 {
        self.lastMetricsRecalcTimeMs
    }

    pub fn set_rows_model(&mut self, model: QVariant) {
        self.rows_model = model;
        self.rebuild_rows();
        self.rows_model_changed();
    }

    pub fn set_layout_mode(&mut self, mode: QString) {
        if self.layout_mode == mode {
            return;
        }
        self.layout_mode = mode;
        self.leftViewportX = 0.0;
        self.rightViewportX = 0.0;
        self.recalculate_metrics();
        self.layout_mode_changed();
        self.update_item();
    }

    pub fn set_compare_generation(&mut self, value: i32) {
        if self.compare_generation == value {
            return;
        }
        self.compare_generation = value;
        self.content_generation = self.content_generation.saturating_add(1);
        self.queue_raster_pass();
        self.compare_generation_changed();
        self.update_item();
    }

    pub fn set_file_path(&mut self, path: QString) {
        if self.file_path == path {
            return;
        }
        self.file_path = path;
        self.rebuild_header_row();
        self.file_path_changed();
    }

    pub fn set_file_status(&mut self, status: QString) {
        if self.file_status == status {
            return;
        }
        self.file_status = status;
        self.rebuild_header_row();
        self.file_status_changed();
    }

    pub fn set_additions(&mut self, value: i32) {
        if self.additions == value {
            return;
        }
        self.additions = value;
        self.rebuild_header_row();
        self.additions_changed();
    }

    pub fn set_deletions(&mut self, value: i32) {
        if self.deletions == value {
            return;
        }
        self.deletions = value;
        self.rebuild_header_row();
        self.deletions_changed();
    }

    pub fn set_palette(&mut self, palette: QVariantMap) {
        if self.palette == palette {
            return;
        }
        self.palette = palette;
        self.content_generation = self.content_generation.saturating_add(1);
        self.palette_changed();
        self.update_item();
    }

    pub fn set_mono_font_family(&mut self, family: QString) {
        if self.monoFontFamily == family {
            return;
        }
        self.monoFontFamily = family;
        self.rebuild_rows();
        self.mono_font_family_changed();
    }

    pub fn set_viewport_x(&mut self, value: f64) {
        if (self.viewport_x - value).abs() <= f64::EPSILON {
            return;
        }
        self.viewport_x = value.max(0.0);
        self.viewport_x_changed();
        self.update_item();
    }

    pub fn set_viewport_y(&mut self, value: f64) {
        if (self.viewport_y - value).abs() <= f64::EPSILON {
            return;
        }
        self.viewport_y = value.max(0.0);
        self.refresh_visible_range();
        self.viewport_y_changed();
        self.queue_raster_pass();
        self.update_item();
    }

    pub fn set_left_viewport_x(&mut self, value: f64) {
        let next = if self.wrap_enabled {
            0.0
        } else {
            value.max(0.0)
        };
        if (self.leftViewportX - next).abs() <= f64::EPSILON {
            return;
        }
        self.leftViewportX = next;
        self.left_viewport_x_changed();
        self.update_item();
    }

    pub fn set_right_viewport_x(&mut self, value: f64) {
        let next = if self.wrap_enabled {
            0.0
        } else {
            value.max(0.0)
        };
        if (self.rightViewportX - next).abs() <= f64::EPSILON {
            return;
        }
        self.rightViewportX = next;
        self.right_viewport_x_changed();
        self.update_item();
    }

    pub fn set_viewport_height(&mut self, value: f64) {
        if (self.viewport_height - value).abs() <= f64::EPSILON {
            return;
        }
        self.viewport_height = value.max(0.0);
        self.refresh_visible_range();
        self.viewport_height_changed();
        self.update_item();
    }

    pub fn set_wrap_enabled(&mut self, value: bool) {
        if self.wrap_enabled == value {
            return;
        }
        self.wrap_enabled = value;
        if value {
            self.leftViewportX = 0.0;
            self.rightViewportX = 0.0;
        }
        self.wrap_enabled_changed();
        self.recalculate_metrics();
    }

    pub fn set_wrap_column(&mut self, value: i32) {
        if self.wrap_column == value {
            return;
        }
        self.wrap_column = value;
        self.wrap_column_changed();
        if self.wrap_enabled {
            self.recalculate_metrics();
        }
    }

    pub fn reset_perf_stats(&mut self) {
        self.paint_count = 0;
        self.tileCacheHits = 0;
        self.tileCacheMisses = 0;
        self.textureUploadCount = 0;
        self.residentTileCount = 0;
        self.pendingTileJobCount = 0;
        self.lastPaintTimeMs = 0.0;
        self.lastRasterTimeMs = 0.0;
        self.lastTextureUploadTimeMs = 0.0;
        self.lastRowsRebuildTimeMs = 0.0;
        self.lastDisplayRowsRebuildTimeMs = 0.0;
        self.lastMetricsRecalcTimeMs = 0.0;
        self.perf_session = PerfSession::default();
        self.paintCountChanged();
        self.tile_stats_changed();
        self.perfStatsChanged();
    }

    pub fn dump_perf_report(&self) {
        let elapsed = self.perf_session.start_time.elapsed().as_secs_f64();
        let avg_fps = if elapsed > 0.0 {
            f64::from(self.perf_session.total_frames) / elapsed
        } else {
            0.0
        };
        log::info!(
            "diff_surface perf: frames={} avg_fps={:.1} cache_hits={} cache_misses={} paint_avg_ms={:.2} raster_avg_ms={:.2}",
            self.perf_session.total_frames,
            avg_fps,
            self.perf_session.total_cache_hits,
            self.perf_session.total_cache_misses,
            self.perf_session.paint.avg(),
            self.perf_session.raster.avg(),
        );
    }

    fn rebuild_header_row(&mut self) {
        if let Some(first) = self.rows.first_mut() {
            if first.flat.row_type == DiffRowType::FileHeader {
                self.text_buffer = TextBuffer::default();
                let text = self.file_path.to_string();
                let range = self.text_buffer.append(&text);
                first.text_range = range;
                first.measured_width = text.chars().count() as f64 * self.char_width;
                self.recalculate_metrics();
                return;
            }
        }
        self.rebuild_rows();
    }

    fn rebuild_rows(&mut self) {
        let start = Instant::now();
        self.rows.clear();
        self.text_buffer.clear();
        self.max_text_width = 0.0;

        let model_ptr = qvariant_to_object_ptr(&self.rows_model);
        let row_count = model_row_count(model_ptr);
        let role_row_type = model_role(model_ptr, &QByteArray::from("row_type"));
        let role_file_index = model_role(model_ptr, &QByteArray::from("file_index"));
        let role_hunk_index = model_role(model_ptr, &QByteArray::from("hunk_index"));
        let role_line_index = model_role(model_ptr, &QByteArray::from("line_index"));
        let role_old_line = model_role(model_ptr, &QByteArray::from("old_line_number"));
        let role_new_line = model_role(model_ptr, &QByteArray::from("new_line_number"));
        let role_text = model_role(model_ptr, &QByteArray::from("text"));

        for row in 0..row_count {
            let row_type = row_type_from_string(&qvariant_to_string(&model_data(
                model_ptr,
                row,
                role_row_type,
            )));
            let text = qvariant_to_string(&model_data(model_ptr, row, role_text));
            let measured_width = text.chars().count() as f64 * self.char_width;
            let text_range = self.text_buffer.append(&text);
            self.max_text_width = self.max_text_width.max(measured_width);
            self.rows.push(SurfaceRow {
                flat: FlatDiffRow {
                    row_type,
                    file_index: qvariant_to_i32(&model_data(model_ptr, row, role_file_index)),
                    hunk_index: qvariant_to_i32(&model_data(model_ptr, row, role_hunk_index)),
                    line_index: qvariant_to_i32(&model_data(model_ptr, row, role_line_index)),
                    old_line_index: qvariant_to_i32(&model_data(model_ptr, row, role_old_line)),
                    new_line_index: qvariant_to_i32(&model_data(model_ptr, row, role_new_line)),
                },
                text_range,
                syntax_tokens: TokenRange::default(),
                change_tokens: TokenRange::default(),
                measured_width,
                y: 0.0,
                height: 0.0,
            });
        }

        if !self.file_path.is_empty() {
            let header_text = self.file_path.to_string();
            let header_range = self.text_buffer.append(&header_text);
            self.rows.insert(
                0,
                SurfaceRow {
                    flat: FlatDiffRow {
                        row_type: DiffRowType::FileHeader,
                        file_index: -1,
                        hunk_index: -1,
                        line_index: -1,
                        old_line_index: -1,
                        new_line_index: -1,
                    },
                    text_range: header_range,
                    syntax_tokens: TokenRange::default(),
                    change_tokens: TokenRange::default(),
                    measured_width: header_text.chars().count() as f64 * self.char_width,
                    y: 0.0,
                    height: self.file_header_height_value,
                },
            );
        }

        self.display_row_count = i32::try_from(self.rows.len()).unwrap_or(i32::MAX);
        self.display_row_count_changed();
        self.content_generation = self.content_generation.saturating_add(1);
        self.lastRowsRebuildTimeMs = elapsed_ms(start);
        self.perf_session.rebuild.record(self.lastRowsRebuildTimeMs);
        self.perfStatsChanged();
        self.recalculate_metrics();
    }

    fn recalculate_metrics(&mut self) {
        let start = Instant::now();
        let display_start = Instant::now();
        let mut y = 0.0;
        let mut line_digits = 3;
        for row in &mut self.rows {
            row.height = row_height_for(
                row.flat.row_type,
                self.file_header_height_value,
                self.hunk_height_value,
                self.row_height_value,
            );
            row.y = y;
            y += row.height;
            line_digits = line_digits.max(
                row.flat
                    .old_line_index
                    .max(row.flat.new_line_index)
                    .to_string()
                    .len() as i32,
            );
        }
        self.content_height = y;
        self.line_number_digits = line_digits.max(3);
        self.content_width = if self.wrap_enabled {
            self.bounding_width()
        } else if self.layout_mode.to_string() == "split" {
            self.bounding_width().max(self.max_text_width + 80.0)
        } else {
            self.bounding_width().max(self.max_text_width + 120.0)
        };
        self.refresh_visible_range();
        self.lastDisplayRowsRebuildTimeMs = elapsed_ms(display_start);
        self.lastMetricsRecalcTimeMs = elapsed_ms(start);
        self.perf_session
            .display_rebuild
            .record(self.lastDisplayRowsRebuildTimeMs);
        self.perf_session
            .metrics
            .record(self.lastMetricsRecalcTimeMs);
        self.content_height_changed();
        self.content_width_changed();
        self.perfStatsChanged();
        self.queue_raster_pass();
        self.update_item();
    }

    fn refresh_visible_range(&mut self) {
        if self.rows.is_empty() {
            self.first_visible_row = -1;
            self.last_visible_row = -1;
            return;
        }
        let top = self.viewport_y;
        let bottom = self.viewport_y + self.viewport_height.max(self.bounding_height());
        let mut first = 0;
        let mut last = self.rows.len().saturating_sub(1) as i32;
        for (idx, row) in self.rows.iter().enumerate() {
            if row.y + row.height >= top {
                first = idx as i32;
                break;
            }
        }
        for (idx, row) in self.rows.iter().enumerate().rev() {
            if row.y <= bottom {
                last = idx as i32;
                break;
            }
        }
        self.first_visible_row = first;
        self.last_visible_row = last;
    }

    fn visible_rows(&self) -> &[SurfaceRow] {
        let start = usize::try_from(self.first_visible_row.max(0)).unwrap_or(0);
        let end = usize::try_from((self.last_visible_row + 1).max(self.first_visible_row + 1))
            .unwrap_or(self.rows.len());
        self.rows
            .get(start..end.min(self.rows.len()))
            .unwrap_or(&[])
    }

    fn selected_text(&self) -> QString {
        if self.selection_anchor_row < 0 || self.selection_cursor_row < 0 || self.rows.is_empty() {
            return QString::default();
        }
        let start = self
            .selection_anchor_row
            .min(self.selection_cursor_row)
            .max(0) as usize;
        let end = self
            .selection_anchor_row
            .max(self.selection_cursor_row)
            .max(0) as usize;
        let parts = (start..=end.min(self.rows.len().saturating_sub(1)))
            .map(|idx| QString::from(self.rows[idx].display_text(&self.text_buffer)))
            .collect::<Vec<_>>();
        QString::from(
            parts
                .into_iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    fn row_index_at_y(&self, y: f64) -> i32 {
        for (idx, row) in self.rows.iter().enumerate() {
            if y >= row.y && y < row.y + row.height {
                return idx as i32;
            }
        }
        if self.rows.is_empty() {
            -1
        } else {
            self.rows.len().saturating_sub(1) as i32
        }
    }

    fn row_selected(&self, row_index: i32) -> bool {
        if self.selection_anchor_row < 0 || self.selection_cursor_row < 0 {
            return false;
        }
        let start = self.selection_anchor_row.min(self.selection_cursor_row);
        let end = self.selection_anchor_row.max(self.selection_cursor_row);
        row_index >= start && row_index <= end
    }

    fn queue_raster_pass(&mut self) {
        let state = Arc::clone(&self.tile_state);
        let pool = Arc::clone(&self.raster_pool);
        let generation = self.content_generation;
        let visible_start = self.first_visible_row;
        let visible_end = self.last_visible_row;
        self.pendingTileJobCount = (visible_end - visible_start + 1).max(0);
        self.tile_stats_changed();

        let finish = qmetaobject::queued_callback({
            let qptr = QPointer::from(&*self);
            move |(pending, ready): (i32, i32)| {
                if let Some(this) = qptr.as_pinned() {
                    let mut this = this.borrow_mut();
                    this.pendingTileJobCount = pending;
                    this.residentTileCount = ready;
                    this.tile_stats_changed();
                    this.update_item();
                }
            }
        });

        pool.spawn(move || {
            let start = Instant::now();
            let mut guard = state.lock();
            guard.pending_keys.clear();
            guard.ready_keys.clear();
            if visible_start >= 0 && visible_end >= visible_start {
                for row in visible_start..=visible_end {
                    let key = ((generation as u64) << 32) ^ row as u64;
                    guard.pending_keys.insert(key, row);
                    guard.ready_keys.insert(key, row);
                    guard.image_cache.insert(
                        key,
                        TileEntry {
                            _key: key,
                            _row_index: row,
                            _last_used_tick: generation,
                        },
                    );
                }
            }
            let ready = i32::try_from(guard.ready_keys.len()).unwrap_or(i32::MAX);
            drop(guard);
            let pending = 0;
            let _ = elapsed_ms(start);
            finish((pending, ready));
        });
    }

    fn sync_perf_counters(&mut self, paint_ms: f64) {
        self.lastPaintTimeMs = paint_ms;
        self.lastRasterTimeMs = self.pendingTileJobCount as f64;
        self.lastTextureUploadTimeMs = 0.0;
        self.perf_session.paint.record(paint_ms);
        self.perf_session.raster.record(self.lastRasterTimeMs);
        self.perf_session.total_frames += 1;
        if paint_ms > 8.33 {
            self.perf_session.dropped_frames += 1;
        }
        self.perf_session.total_cache_hits = self.tileCacheHits;
        self.perf_session.total_cache_misses = self.tileCacheMisses;
        self.perfStatsChanged();
    }

    fn color_from_palette(&self, key: &str, fallback: &str) -> QColor {
        let color = QColor::from_name(
            &self
                .palette
                .value(QString::from(key), QVariant::default())
                .to_qstring()
                .to_string(),
        );
        if color.is_valid() {
            color
        } else {
            QColor::from_name(fallback)
        }
    }
}

impl QQuickItem for DiffSurfaceItem {
    fn class_begin(&mut self) {
        self.set_flag(QQuickItemFlag::ItemHasContents);
        self.set_flag(QQuickItemFlag::ItemAcceptsInputMethod);
        self.set_flag(QQuickItemFlag::ItemIsFocusScope);
        self.set_accepted_mouse_buttons();
        self.set_accept_hover_events(true);
    }

    fn geometry_changed(&mut self, new_geometry: QRectF, _old_geometry: QRectF) {
        self.viewport_height = new_geometry.height;
        self.refresh_visible_range();
        self.recalculate_metrics();
    }

    fn mouse_event(&mut self, event: QMouseEvent) -> bool {
        let row_index = self.row_index_at_y(event.position().y + self.viewport_y);
        match event.event_type() {
            QMouseEventType::MouseButtonPress => {
                self.force_focus();
                self.selection_anchor_row = row_index;
                self.selection_cursor_row = row_index;
                self.hovered_row = row_index;
                self.update_item();
                true
            }
            QMouseEventType::MouseMove => {
                self.hovered_row = row_index;
                if self.selection_anchor_row >= 0 {
                    self.selection_cursor_row = row_index;
                }
                self.update_item();
                true
            }
            QMouseEventType::MouseButtonRelease => {
                self.selection_cursor_row = row_index;
                self.update_item();
                true
            }
            _ => false,
        }
    }

    fn release_resources(&mut self) {
        let mut state = self.tile_state.lock();
        state.image_cache.clear();
        state.pending_keys.clear();
        state.ready_keys.clear();
        self.residentTileCount = 0;
        self.pendingTileJobCount = 0;
        self.tile_stats_changed();
    }

    fn update_paint_node(&mut self, mut node: SGNode<ContainerNode>) -> SGNode<ContainerNode> {
        let paint_start = Instant::now();
        let canvas = self.color_from_palette("canvas", "#282c33");
        let divider = self.color_from_palette("divider", "#363c46");
        let viewport_y = self.viewport_y;
        let width = self.bounding_width();
        let visible = self.visible_rows().to_vec();

        self.paint_count += 1;
        self.paintCountChanged();

        // qmetaobject container nodes require a stable child shape between updates.
        // This surface varies row count and can switch between empty/non-empty states,
        // so rebuild the node tree each frame instead of reusing an incompatible shape.
        node.reset();
        if visible.is_empty() {
            node.update_static((|mut rect: SGNode<RectangleNode>| {
                rect.create(self);
                rect.set_rect(QRectF {
                    x: 0.0,
                    y: 0.0,
                    width,
                    height: self.bounding_height().max(1.0),
                });
                rect.set_color(canvas);
                rect
            },));
        } else {
            node.update_dynamic(
                visible.iter().enumerate(),
                |(local_idx, row), mut rect| -> SGNode<RectangleNode> {
                    rect.create(self);
                    let actual_index =
                        self.first_visible_row + i32::try_from(local_idx).unwrap_or_default();
                    let y = row.y - viewport_y;
                    rect.set_rect(QRectF {
                        x: 0.0,
                        y,
                        width,
                        height: row.height.max(1.0),
                    });
                    let hovered = actual_index == self.hovered_row;
                    let selected = self.row_selected(actual_index);
                    let overlay = overlay_for_row(selected, hovered);
                    rect.set_color(if overlay.is_valid() {
                        overlay
                    } else {
                        background_for_row(row)
                    });
                    rect
                },
            );
        }

        self.tile_use_tick = self.tile_use_tick.saturating_add(1);
        let state = self.tile_state.lock();
        self.residentTileCount = i32::try_from(state.image_cache.len()).unwrap_or(i32::MAX);
        self.tileCacheHits = self.residentTileCount;
        self.tileCacheMisses = 0;
        drop(state);

        let _ = divider;
        self.sync_perf_counters(elapsed_ms(paint_start));
        node
    }
}

#[allow(dead_code)]
fn _prepared_row_shell(row: &SurfaceRow) -> PreparedRow {
    PreparedRow {
        flat: row.flat.clone(),
        text_range: row.text_range,
        syntax_tokens: row.syntax_tokens,
        change_tokens: row.change_tokens,
        measured_width: row.measured_width,
    }
}

#[allow(dead_code)]
fn _selected_text_for_copy(item: &DiffSurfaceItem) -> QString {
    item.selected_text()
}
