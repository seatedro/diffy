use std::ffi::c_void;
use std::sync::Arc;
use std::time::Instant;

use qmetaobject::prelude::*;
use qmetaobject::scenegraph::{ContainerNode, SGNode};
use qmetaobject::{QMouseEvent, QMouseEventType, QQuickItem, QVariantMap};
use qttypes::{ImageFormat, QColor, QImage, QRectF, QSize};

use crate::app::surface::display_layout::{
    DisplayLayoutConfig, DisplayLayoutMetrics, rebuild_display_rows,
};
use crate::app::surface::render_doc::{DisplayRow, RenderDoc, clone_render_doc};
use crate::app::surface::strip_layout::{StripLayout, build_strip_layouts, visible_strip_range};
use crate::app::theme::default_mono_family;

const BODY_FONT_PX: i32 = 12;
const ROW_VERTICAL_PADDING_PX: u32 = 6;
const HEADER_PADDING_PX: u32 = 10;
const HUNK_PADDING_PX: u32 = 8;
const STRIP_HEIGHT_PX: u32 = 384;
const STRIP_OVERSCAN: i32 = 1;
const UNIFIED_TEXT_PADDING_PX: f64 = 10.0;
const SPLIT_TEXT_PADDING_PX: f64 = 8.0;
const SPLIT_GAP_PX: f64 = 16.0;

cpp! {{
    #include <QtCore/QByteArray>
    #include <QtCore/QString>
    #include <QtGui/QColor>
    #include <QtGui/QFont>
    #include <QtGui/QFontMetricsF>
    #include <QtGui/QImage>
    #include <QtGui/QPainter>
    #include <QtGui/QTextLayout>
    #include <QtGui/QTextOption>
    #include <QtQuick/QQuickItem>
    #include <QtQuick/QQuickWindow>
    #include <QtQuick/QSGNode>
    #include <QtQuick/QSGSimpleTextureNode>
    #include <QtQuick/QSGTexture>

    struct DiffFontMetrics {
        double char_width;
        double line_height;
        double ascent;
    };

    struct DiffByteRange {
        quint32 start;
        quint32 len;
    };

    struct DiffRunRange {
        quint32 start;
        quint32 len;
    };

    struct DiffStyleRun {
        quint32 byte_start;
        quint32 byte_len;
        quint16 style_id;
        quint16 flags;
    };

    struct DiffRenderLine {
        quint8 kind;
        quint8 flags;
        quint16 reserved;
        quint32 old_line_no;
        quint32 new_line_no;
        quint32 left_cols;
        quint32 right_cols;
        DiffByteRange left_text;
        DiffByteRange right_text;
        DiffRunRange left_runs;
        DiffRunRange right_runs;
    };

    struct DiffDisplayRow {
        quint32 line_index;
        quint32 y_px;
        quint16 h_px;
        quint16 wrap_left;
        quint16 wrap_right;
        quint8 kind;
        quint8 reserved0;
        quint8 reserved1;
        quint8 reserved2;
    };

    class DiffStripNode : public QSGSimpleTextureNode {
    public:
        DiffStripNode() {
            setOwnsTexture(false);
        }
    };

    static QSGNode *diffyEnsureRoot(QSGNode *root) {
        if (!root) {
            root = new QSGNode();
        }
        return root;
    }

    static DiffStripNode *diffyEnsureChild(QSGNode *root, int index) {
        int i = 0;
        for (auto *child = root->firstChild(); child; child = child->nextSibling(), ++i) {
            if (i == index) {
                return static_cast<DiffStripNode *>(child);
            }
        }

        while (i <= index) {
            auto *node = new DiffStripNode();
            root->appendChildNode(node);
            if (i == index) {
                return node;
            }
            ++i;
        }

        return nullptr;
    }

    static void diffyTrimChildren(QSGNode *root, int active_count) {
        int i = 0;
        auto *child = root->firstChild();
        while (child) {
            auto *next = child->nextSibling();
            if (i >= active_count) {
                root->removeChildNode(child);
                delete child;
            }
            child = next;
            ++i;
        }
    }

    static void diffySyncChild(QSGNode *root, int index, QRectF rect, QSGTexture *texture) {
        auto *node = diffyEnsureChild(root, index);
        if (!node) {
            return;
        }
        node->setTexture(texture);
        node->setRect(rect);
    }

    static void diffyDeleteTexture(QSGTexture *texture) {
        delete texture;
    }

    static QSGTexture *diffyCreateTexture(QQuickItem *item, QImage *image) {
        if (!item || !image) {
            return nullptr;
        }
        auto *window = item->window();
        if (!window) {
            return nullptr;
        }
        auto *texture = window->createTextureFromImage(*image);
        if (texture) {
            texture->setFiltering(QSGTexture::Nearest);
        }
        return texture;
    }

    static double diffyEffectiveDevicePixelRatio(QQuickItem *item) {
        if (!item) {
            return 1.0;
        }
        auto *window = item->window();
        if (!window) {
            return 1.0;
        }
        return window->effectiveDevicePixelRatio();
    }

    static void diffySetImageDevicePixelRatio(QImage *image, double dpr) {
        if (!image) {
            return;
        }
        image->setDevicePixelRatio(dpr > 0.0 ? dpr : 1.0);
    }

    static DiffFontMetrics diffyMeasureFontMetrics(QString family, int pixel_size) {
        QFont font(family);
        font.setStyleHint(QFont::TypeWriter);
        font.setFixedPitch(true);
        font.setPixelSize(pixel_size);
        QFontMetricsF metrics(font);
        return DiffFontMetrics {
            metrics.horizontalAdvance(QStringLiteral("M")),
            metrics.height(),
            metrics.ascent(),
        };
    }

    static QColor diffySyntaxColor(
        quint16 style_id,
        QColor text_base,
        QColor text_muted,
        QColor text_strong,
        QColor accent,
        QColor accent_strong,
        QColor success_text,
        QColor warning_text
    ) {
        switch (style_id) {
        case 1: return accent_strong;  // Keyword
        case 2: return success_text;   // String
        case 3: return text_muted;     // Comment
        case 4: return warning_text;   // Number
        case 5: return accent;         // Type
        case 6: return text_strong;    // Function
        case 7: return accent_strong;  // Operator
        case 8: return text_muted;     // Punctuation
        case 9: return accent;         // Variable
        case 10: return warning_text;  // Constant
        case 11: return accent_strong; // Builtin
        case 12: return accent;        // Attribute
        case 13: return success_text;  // Tag
        case 14: return accent;        // Property
        case 15: return accent;        // Namespace
        case 16: return warning_text;  // Label
        case 17: return warning_text;  // Preprocessor
        default: return text_base;
        }
    }

    static QString diffySliceUtf8(const unsigned char *bytes, DiffByteRange range) {
        if (!bytes || range.start == quint32(-1) || range.len == 0) {
            return QString();
        }
        return QString::fromUtf8(
            reinterpret_cast<const char *>(bytes + range.start),
            int(range.len)
        );
    }

    static QVector<QTextLayout::FormatRange> diffyBuildFormatRanges(
        const unsigned char *bytes,
        DiffByteRange text_range,
        const DiffStyleRun *runs,
        DiffRunRange run_range,
        QColor text_base,
        QColor text_muted,
        QColor text_strong,
        QColor accent,
        QColor accent_strong,
        QColor success_text,
        QColor warning_text,
        QColor change_bg
    ) {
        QVector<QTextLayout::FormatRange> formats;
        if (!bytes || text_range.start == quint32(-1) || run_range.len == 0) {
            return formats;
        }
        formats.reserve(int(run_range.len));

        for (quint32 i = 0; i < run_range.len; ++i) {
            const auto &run = runs[run_range.start + i];
            QTextLayout::FormatRange format;
            format.start = QString::fromUtf8(
                reinterpret_cast<const char *>(bytes + text_range.start),
                int(run.byte_start)
            ).size();
            format.length = QString::fromUtf8(
                reinterpret_cast<const char *>(bytes + text_range.start + run.byte_start),
                int(run.byte_len)
            ).size();

            QTextCharFormat char_format;
            char_format.setForeground(diffySyntaxColor(
                run.style_id,
                text_base,
                text_muted,
                text_strong,
                accent,
                accent_strong,
                success_text,
                warning_text
            ));
            if ((run.flags & 0x1u) != 0) {
                char_format.setBackground(change_bg);
            }
            format.format = char_format;
            formats.push_back(format);
        }

        return formats;
    }

    static void diffyDrawStyledBlock(
        QPainter &painter,
        const unsigned char *bytes,
        const DiffRenderLine &line,
        bool left_side,
        const DiffStyleRun *runs,
        qreal x,
        qreal y,
        qreal width,
        QString family,
        int font_px,
        QColor text_base,
        QColor text_muted,
        QColor text_strong,
        QColor accent,
        QColor accent_strong,
        QColor success_text,
        QColor warning_text,
        QColor change_bg,
        bool wrap_enabled
    ) {
        const auto text_range = left_side ? line.left_text : line.right_text;
        const auto run_range = left_side ? line.left_runs : line.right_runs;
        if (text_range.start == quint32(-1) || text_range.len == 0 || width <= 1.0) {
            return;
        }

        QFont font(family);
        font.setStyleHint(QFont::TypeWriter);
        font.setFixedPitch(true);
        font.setPixelSize(font_px);

        QTextOption option;
        option.setWrapMode(wrap_enabled ? QTextOption::WrapAnywhere : QTextOption::NoWrap);

        QTextLayout layout(diffySliceUtf8(bytes, text_range), font);
        layout.setTextOption(option);
        layout.setFormats(diffyBuildFormatRanges(
            bytes,
            text_range,
            runs,
            run_range,
            text_base,
            text_muted,
            text_strong,
            accent,
            accent_strong,
            success_text,
            warning_text,
            change_bg
        ));

        layout.beginLayout();
        qreal line_y = 0.0;
        while (true) {
            QTextLine text_line = layout.createLine();
            if (!text_line.isValid()) {
                break;
            }
            text_line.setLineWidth(width);
            text_line.setPosition(QPointF(0.0, line_y));
            line_y += text_line.height();
        }
        layout.endLayout();

        painter.save();
        painter.translate(x, y);
        layout.draw(&painter, QPointF(0.0, 0.0));
        painter.restore();
    }

    static QColor diffyUnifiedRowColor(
        quint8 kind,
        int row_index,
        QColor panel_strong,
        QColor panel_tint,
        QColor line_context,
        QColor line_context_alt,
        QColor line_add,
        QColor line_del
    ) {
        switch (kind) {
        case 0: return panel_strong;
        case 1: return panel_tint;
        case 3: return line_add;
        case 4: return line_del;
        case 5: return line_context_alt;
        default: return (row_index & 1) == 0 ? line_context : line_context_alt;
        }
    }

    static void diffyRasterStrip(
        QImage *image,
        const DiffDisplayRow *rows,
        quint32 row_count,
        quint32 first_row_index,
        const DiffRenderLine *lines,
        const DiffStyleRun *runs,
        const unsigned char *text_bytes,
        bool split_mode,
        bool wrap_enabled,
        quint32 viewport_x,
        quint32 viewport_y,
        quint32 strip_top,
        quint32 strip_height,
        quint32 gutter_digits,
        double char_width,
        double line_height_px,
        double body_font_px,
        double unified_text_start_px,
        double unified_text_width_px,
        double split_side_width_px,
        double split_text_start_px,
        double split_text_width_px,
        QString family,
        QColor canvas,
        QColor divider,
        QColor panel_strong,
        QColor panel_tint,
        QColor text_base,
        QColor text_muted,
        QColor text_strong,
        QColor accent,
        QColor accent_strong,
        QColor success_text,
        QColor warning_text,
        QColor selection_bg,
        QColor line_context,
        QColor line_context_alt,
        QColor line_add,
        QColor line_add_accent,
        QColor line_del,
        QColor line_del_accent,
        int hovered_row,
        int selection_start,
        int selection_end
    ) {
        if (!image) {
            return;
        }

        image->fill(canvas);

        QPainter painter(image);
        painter.setRenderHint(QPainter::TextAntialiasing, true);
        painter.setRenderHint(QPainter::Antialiasing, false);

        QFont font(family);
        font.setStyleHint(QFont::TypeWriter);
        font.setFixedPitch(true);
        font.setPixelSize(int(body_font_px));
        painter.setFont(font);

        QFontMetricsF metrics(font);
        const qreal number_width = qreal(gutter_digits) * char_width + 12.0;
        const qreal split_gap = 16.0;

        for (quint32 i = 0; i < row_count; ++i) {
            const auto &row = rows[i];
            const auto &line = lines[row.line_index];
            const int global_row = int(first_row_index + i);
            const bool selected =
                selection_start >= 0 && selection_end >= selection_start &&
                global_row >= selection_start && global_row <= selection_end;
            const bool hovered = hovered_row == global_row;
            const qreal top = qreal(row.y_px - strip_top);
            const qreal height = qreal(row.h_px);

            if (split_mode && line.kind >= 2) {
                const qreal left_width = split_side_width_px;
                const qreal right_x = split_side_width_px + split_gap;

                QColor left_bg = canvas;
                QColor right_bg = canvas;
                switch (line.kind) {
                case 2: left_bg = line_context; right_bg = line_context; break;
                case 3: right_bg = line_add; break;
                case 4: left_bg = line_del; break;
                case 5: left_bg = line_del; right_bg = line_add; break;
                default: break;
                }
                if (selected || hovered) {
                    left_bg = selection_bg;
                    right_bg = selection_bg;
                }

                painter.fillRect(QRectF(0.0, top, left_width, height), left_bg);
                painter.fillRect(QRectF(right_x, top, left_width, height), right_bg);
                painter.fillRect(QRectF(split_side_width_px, top, split_gap, height), canvas);
                painter.setPen(divider);
                painter.drawLine(
                    QPointF(split_side_width_px + split_gap * 0.5, top),
                    QPointF(split_side_width_px + split_gap * 0.5, top + height)
                );

                painter.setPen(text_muted);
                if (line.old_line_no != quint32(-1)) {
                    painter.drawText(
                        QRectF(0.0, top, number_width, height),
                        Qt::AlignRight | Qt::AlignVCenter,
                        QString::number(int(line.old_line_no))
                    );
                }
                if (line.new_line_no != quint32(-1)) {
                    painter.drawText(
                        QRectF(right_x, top, number_width, height),
                        Qt::AlignRight | Qt::AlignVCenter,
                        QString::number(int(line.new_line_no))
                    );
                }

                diffyDrawStyledBlock(
                    painter,
                    text_bytes,
                    line,
                    true,
                    runs,
                    split_text_start_px,
                    top + 2.0,
                    split_text_width_px,
                    family,
                    int(body_font_px),
                    text_base,
                    text_muted,
                    text_strong,
                    accent,
                    accent_strong,
                    success_text,
                    warning_text,
                    line_del_accent,
                    wrap_enabled
                );

                diffyDrawStyledBlock(
                    painter,
                    text_bytes,
                    line,
                    false,
                    runs,
                    right_x + split_text_start_px,
                    top + 2.0,
                    split_text_width_px,
                    family,
                    int(body_font_px),
                    text_base,
                    text_muted,
                    text_strong,
                    accent,
                    accent_strong,
                    success_text,
                    warning_text,
                    line_add_accent,
                    wrap_enabled
                );
            } else {
                painter.fillRect(
                    QRectF(0.0, top, image->width(), height),
                    selected || hovered
                        ? selection_bg
                        : diffyUnifiedRowColor(
                              line.kind,
                              global_row,
                              panel_strong,
                              panel_tint,
                              line_context,
                              line_context_alt,
                              line_add,
                              line_del
                          )
                );

                if (line.kind == 0 || line.kind == 1) {
                    painter.setPen(line.kind == 0 ? text_strong : text_muted);
                    const auto text = diffySliceUtf8(text_bytes, line.left_text);
                    painter.drawText(
                        QPointF(10.0 - qreal(viewport_x), top + metrics.ascent() + 6.0),
                        text
                    );
                } else {
                    painter.setPen(text_muted);
                    if (line.old_line_no != quint32(-1)) {
                        painter.drawText(
                            QRectF(0.0, top, number_width, height),
                            Qt::AlignRight | Qt::AlignVCenter,
                            QString::number(int(line.old_line_no))
                        );
                    }
                    if (line.new_line_no != quint32(-1)) {
                        painter.drawText(
                            QRectF(number_width, top, number_width, height),
                            Qt::AlignRight | Qt::AlignVCenter,
                            QString::number(int(line.new_line_no))
                        );
                    }

                    const bool prefer_left = line.right_text.start == quint32(-1);
                    diffyDrawStyledBlock(
                        painter,
                        text_bytes,
                        line,
                        prefer_left,
                        runs,
                        unified_text_start_px - qreal(viewport_x),
                        top + 2.0,
                        unified_text_width_px,
                        family,
                        int(body_font_px),
                        text_base,
                        text_muted,
                        text_strong,
                        accent,
                        accent_strong,
                        success_text,
                        warning_text,
                        prefer_left ? line_del_accent : line_add_accent,
                        wrap_enabled
                    );
                }
            }
        }

        painter.end();
        (void)viewport_y;
        (void)strip_height;
        (void)line_height_px;
    }
}}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct FontMetrics {
    char_width: f64,
    line_height: f64,
    ascent: f64,
}

#[derive(Default)]
struct StripSlot {
    strip_id: u32,
    top_px: u32,
    logical_height_px: u32,
    image_width_px: i32,
    image_height_px: i32,
    image_dpr: f64,
    row_start: usize,
    row_end: usize,
    rendered_version: u64,
    image: QImage,
    texture_raw: *mut c_void,
}

fn slot_needs_raster(slot: &StripSlot, strip: StripLayout, render_version: u64) -> bool {
    slot.rendered_version != render_version
        || slot.texture_raw.is_null()
        || slot.strip_id != strip.strip_id
        || slot.top_px != strip.top_px
        || slot.logical_height_px != strip.height_px
        || slot.row_start != strip.row_start
        || slot.row_end != strip.row_end
}

#[allow(non_snake_case)]
#[derive(QObject)]
pub struct DiffSurfaceItem {
    base: qt_base_class!(trait QQuickItem),

    render_key: qt_property!(i64; WRITE set_render_key NOTIFY render_key_changed ALIAS renderKey),
    render_key_changed: qt_signal!(),

    layout_mode: qt_property!(QString; WRITE set_layout_mode NOTIFY layout_mode_changed ALIAS layoutMode),
    layout_mode_changed: qt_signal!(),

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

    strip_count: qt_property!(i32; READ get_strip_count NOTIFY perfStatsChanged ALIAS stripCount),
    strip_reuse_count: qt_property!(i32; READ get_strip_reuse_count NOTIFY perfStatsChanged ALIAS stripReuseCount),
    strip_reraster_count: qt_property!(i32; READ get_strip_reraster_count NOTIFY perfStatsChanged ALIAS stripRerasterCount),
    lastPaintTimeMs: qt_property!(f64; READ get_last_paint_time_ms NOTIFY perfStatsChanged ALIAS lastPaintTimeMs),
    lastRasterTimeMs: qt_property!(f64; READ get_last_raster_time_ms NOTIFY perfStatsChanged ALIAS lastRasterTimeMs),
    lastLayoutTimeMs: qt_property!(f64; READ get_last_layout_time_ms NOTIFY perfStatsChanged ALIAS lastLayoutTimeMs),
    perfStatsChanged: qt_signal!(),

    scrollToYRequested: qt_signal!(value: f64),
    nextFileRequested: qt_signal!(),
    previousFileRequested: qt_signal!(),

    doc: Option<Arc<RenderDoc>>,
    display_rows_store: Vec<DisplayRow>,
    strip_layouts_store: Vec<StripLayout>,
    strip_slots: Vec<StripSlot>,
    active_slots: Vec<usize>,
    slot_marks: Vec<bool>,
    stale_textures: Vec<*mut c_void>,

    char_width: f64,
    line_height_px: u16,
    font_ascent_px: f64,
    body_row_height_px: u16,
    file_header_height_px: u16,
    hunk_height_px: u16,
    gutter_digits: u32,

    hovered_row: i32,
    selection_anchor_row: i32,
    selection_cursor_row: i32,
    render_version: u64,

    paint_count_value: i32,
    strip_count_value: i32,
    strip_reuse_count_value: i32,
    strip_reraster_count_value: i32,
    last_paint_time_ms_value: f64,
    last_raster_time_ms_value: f64,
    last_layout_time_ms_value: f64,
}

#[repr(C)]
enum QQuickItemFlag {
    ItemHasContents = 0x08,
    ItemAcceptsInputMethod = 0x02,
    ItemIsFocusScope = 0x04,
}

impl Default for DiffSurfaceItem {
    fn default() -> Self {
        let mut this = Self {
            base: Default::default(),
            render_key: 0,
            render_key_changed: Default::default(),
            layout_mode: QString::from("unified"),
            layout_mode_changed: Default::default(),
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
            strip_count: 0,
            strip_reuse_count: 0,
            strip_reraster_count: 0,
            lastPaintTimeMs: 0.0,
            lastRasterTimeMs: 0.0,
            lastLayoutTimeMs: 0.0,
            perfStatsChanged: Default::default(),
            scrollToYRequested: Default::default(),
            nextFileRequested: Default::default(),
            previousFileRequested: Default::default(),
            doc: None,
            display_rows_store: Vec::new(),
            strip_layouts_store: Vec::new(),
            strip_slots: Vec::new(),
            active_slots: Vec::new(),
            slot_marks: Vec::new(),
            stale_textures: Vec::new(),
            char_width: 8.0,
            line_height_px: 16,
            font_ascent_px: 12.0,
            body_row_height_px: 20,
            file_header_height_px: 32,
            hunk_height_px: 24,
            gutter_digits: 3,
            hovered_row: -1,
            selection_anchor_row: -1,
            selection_cursor_row: -1,
            render_version: 1,
            paint_count_value: 0,
            strip_count_value: 0,
            strip_reuse_count_value: 0,
            strip_reraster_count_value: 0,
            last_paint_time_ms_value: 0.0,
            last_raster_time_ms_value: 0.0,
            last_layout_time_ms_value: 0.0,
        };
        this.refresh_font_metrics();
        this
    }
}

impl DiffSurfaceItem {
    fn cpp_item_ptr(&self) -> *mut c_void {
        self.get_cpp_object()
    }

    fn set_flag(&mut self, flag: QQuickItemFlag) {
        let obj = self.cpp_item_ptr();
        cpp!(unsafe [obj as "QQuickItem*", flag as "QQuickItem::Flag"] {
            if (obj) {
                obj->setFlag(flag, true);
            }
        });
    }

    fn set_accept_hover_events(&mut self, enabled: bool) {
        let obj = self.cpp_item_ptr();
        cpp!(unsafe [obj as "QQuickItem*", enabled as "bool"] {
            if (obj) {
                obj->setAcceptHoverEvents(enabled);
            }
        });
    }

    fn set_accepted_mouse_buttons(&mut self) {
        let obj = self.cpp_item_ptr();
        cpp!(unsafe [obj as "QQuickItem*"] {
            if (obj) {
                obj->setAcceptedMouseButtons(Qt::LeftButton);
            }
        });
    }

    fn force_focus(&mut self) {
        let obj = self.cpp_item_ptr();
        cpp!(unsafe [obj as "QQuickItem*"] {
            if (obj) {
                obj->forceActiveFocus(Qt::MouseFocusReason);
            }
        });
    }

    fn update_item(&self) {
        (self as &dyn QQuickItem).update();
    }

    fn bounding_width(&self) -> f64 {
        (self as &dyn QQuickItem).bounding_rect().width.max(1.0)
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

    fn refresh_font_metrics(&mut self) {
        let family = self.monoFontFamily.clone();
        let metrics = cpp!(unsafe [family as "QString"] -> FontMetrics as "DiffFontMetrics" {
            return diffyMeasureFontMetrics(family, 12);
        });
        self.char_width = metrics.char_width.max(6.0);
        self.line_height_px = metrics.line_height.ceil().max(12.0) as u16;
        self.font_ascent_px = metrics.ascent.max(9.0);
        self.body_row_height_px = self
            .line_height_px
            .saturating_add(ROW_VERTICAL_PADDING_PX as u16);
        self.file_header_height_px = self
            .body_row_height_px
            .saturating_add(HEADER_PADDING_PX as u16);
        self.hunk_height_px = self
            .body_row_height_px
            .saturating_add(HUNK_PADDING_PX as u16);
    }

    fn invalidate_rendering(&mut self) {
        self.render_version = self.render_version.saturating_add(1);
        self.update_item();
    }

    fn selected_bounds(&self) -> Option<(i32, i32)> {
        if self.selection_anchor_row < 0 || self.selection_cursor_row < 0 {
            None
        } else {
            Some((
                self.selection_anchor_row.min(self.selection_cursor_row),
                self.selection_anchor_row.max(self.selection_cursor_row),
            ))
        }
    }

    fn unified_number_width_px(&self) -> f64 {
        f64::from(self.gutter_digits.max(1)) * self.char_width + 12.0
    }

    fn unified_text_start_px(&self) -> f64 {
        self.unified_number_width_px() * 2.0 + 16.0
    }

    fn split_side_width_px(&self) -> f64 {
        ((self.bounding_width() - SPLIT_GAP_PX).max(32.0)) / 2.0
    }

    fn split_number_width_px(&self) -> f64 {
        f64::from(self.gutter_digits.max(1)) * self.char_width + 12.0
    }

    fn split_text_start_px(&self) -> f64 {
        self.split_number_width_px() + 12.0
    }

    fn unified_text_width_px(&self) -> f64 {
        (self.bounding_width() - self.unified_text_start_px() - UNIFIED_TEXT_PADDING_PX).max(1.0)
    }

    fn split_text_width_px(&self) -> f64 {
        (self.split_side_width_px() - self.split_text_start_px() - SPLIT_TEXT_PADDING_PX).max(1.0)
    }

    fn rebuild_display_rows(&mut self) {
        let start = Instant::now();
        self.display_rows_store.clear();
        self.strip_layouts_store.clear();

        let Some(doc) = self.doc.as_ref() else {
            self.display_row_count = 0;
            self.display_row_count_changed();
            self.content_height = 0.0;
            self.content_height_changed();
            self.content_width = self.bounding_width();
            self.content_width_changed();
            self.lastLayoutTimeMs = 0.0;
            self.last_layout_time_ms_value = 0.0;
            self.perfStatsChanged();
            self.invalidate_rendering();
            return;
        };

        let layout_summary = rebuild_display_rows(
            doc,
            DisplayLayoutConfig {
                split_mode: self.layout_mode.to_string() == "split",
                wrap_enabled: self.wrap_enabled,
                wrap_column: self.wrap_column.max(0) as u32,
                char_width_px: self.char_width,
                unified_text_width_px: self.unified_text_width_px(),
                split_text_width_px: self.split_text_width_px(),
            },
            DisplayLayoutMetrics {
                body_row_height_px: self.body_row_height_px,
                file_header_height_px: self.file_header_height_px,
                hunk_height_px: self.hunk_height_px,
            },
            &mut self.display_rows_store,
        );
        self.gutter_digits = layout_summary.gutter_digits;

        build_strip_layouts(
            &self.display_rows_store,
            STRIP_HEIGHT_PX,
            &mut self.strip_layouts_store,
        );

        self.display_row_count = i32::try_from(self.display_rows_store.len()).unwrap_or(i32::MAX);
        self.display_row_count_changed();

        self.content_height = f64::from(layout_summary.content_height_px);
        self.content_height_changed();

        self.content_width = if self.layout_mode.to_string() == "split" || self.wrap_enabled {
            self.bounding_width()
        } else {
            (self.unified_text_start_px()
                + UNIFIED_TEXT_PADDING_PX
                + f64::from(layout_summary.max_cols) * self.char_width)
                .max(self.bounding_width())
        };
        self.content_width_changed();

        let layout_ms = elapsed_ms(start);
        self.lastLayoutTimeMs = layout_ms;
        self.last_layout_time_ms_value = layout_ms;
        self.perfStatsChanged();
        self.invalidate_rendering();
    }

    fn row_index_at_y(&self, y: f64) -> i32 {
        if self.display_rows_store.is_empty() {
            return -1;
        }
        let y_px = y.max(0.0) as u32;
        let index = self
            .display_rows_store
            .partition_point(|row| row.bottom_px() <= y_px);
        if index >= self.display_rows_store.len() {
            (self.display_rows_store.len() as i32).saturating_sub(1)
        } else {
            index as i32
        }
    }

    fn visible_strip_window(&self) -> (usize, usize) {
        if self.strip_layouts_store.is_empty() || self.content_height <= 0.0 {
            return (0, 0);
        }
        let visible = visible_strip_range(
            &self.strip_layouts_store,
            self.viewport_y.max(0.0).floor() as u32,
            self.viewport_height.max(1.0).ceil() as u32,
            STRIP_OVERSCAN.max(0) as usize,
        );
        (visible.start, visible.end.saturating_sub(visible.start))
    }

    fn device_pixel_ratio(&self) -> f64 {
        let item = self.cpp_item_ptr();
        let dpr = cpp!(unsafe [item as "QQuickItem*"] -> f64 as "double" {
            return diffyEffectiveDevicePixelRatio(item);
        });
        dpr.max(1.0)
    }

    fn release_slot_texture(slot: &mut StripSlot) {
        let texture = slot.texture_raw;
        if texture.is_null() {
            return;
        }
        cpp!(unsafe [texture as "QSGTexture*"] {
            diffyDeleteTexture(texture);
        });
        slot.texture_raw = std::ptr::null_mut();
    }

    fn ensure_slot_pool(&mut self, target: usize) {
        if self.strip_slots.len() < target {
            self.strip_slots.resize_with(target, StripSlot::default);
        }
        if self.slot_marks.len() < self.strip_slots.len() {
            self.slot_marks.resize(self.strip_slots.len(), false);
        }
    }

    fn acquire_slot_index(&mut self, strip_id: u32) -> usize {
        for (idx, slot) in self.strip_slots.iter().enumerate() {
            if !self.slot_marks[idx]
                && slot.strip_id == strip_id
                && slot.rendered_version == self.render_version
            {
                self.slot_marks[idx] = true;
                self.strip_reuse_count_value += 1;
                return idx;
            }
        }

        let idx = self
            .slot_marks
            .iter()
            .position(|used| !*used)
            .unwrap_or_else(|| {
                self.strip_slots.push(StripSlot::default());
                self.slot_marks.push(false);
                self.strip_slots.len() - 1
            });
        self.slot_marks[idx] = true;
        idx
    }

    fn rasterize_slot(&mut self, slot_index: usize) {
        let Some(doc) = self.doc.as_ref() else {
            return;
        };
        let (
            slot_row_start,
            slot_row_end,
            slot_top_px,
            slot_logical_height_px,
            slot_image_width_px,
            slot_image_height_px,
            slot_image_dpr,
        ) = {
            let slot = &self.strip_slots[slot_index];
            (
                slot.row_start,
                slot.row_end,
                slot.top_px,
                slot.logical_height_px,
                slot.image_width_px,
                slot.image_height_px,
                slot.image_dpr,
            )
        };
        if slot_row_start >= slot_row_end {
            return;
        }

        let logical_width_px = self.bounding_width().ceil().max(1.0);
        let logical_height_px = f64::from(slot_logical_height_px.max(1));
        let dpr = self.device_pixel_ratio();
        let image_width_px = (logical_width_px * dpr).round().max(1.0) as i32;
        let image_height_px = (logical_height_px * dpr).round().max(1.0) as i32;
        let rows = &self.display_rows_store[slot_row_start..slot_row_end];
        let lines = doc.lines.as_slice();
        let runs = doc.style_runs.as_slice();
        let bytes = doc.text_bytes.as_slice();
        let split_mode = self.layout_mode.to_string() == "split";
        let wrap_enabled = self.wrap_enabled;
        let viewport_x = self.viewport_x.max(0.0) as u32;
        let viewport_y = self.viewport_y.max(0.0) as u32;
        let strip_top = slot_top_px;
        let strip_height = slot_logical_height_px;
        let gutter_digits = self.gutter_digits;
        let char_width = self.char_width;
        let line_height_px = f64::from(self.line_height_px);
        let body_font_px = BODY_FONT_PX as f64;
        let unified_text_start_px = self.unified_text_start_px();
        let unified_text_width_px = self.unified_text_width_px();
        let split_side_width_px = self.split_side_width_px();
        let split_text_start_px = self.split_text_start_px();
        let split_text_width_px = self.split_text_width_px();

        let canvas = self.color_from_palette("canvas", "#20242b");
        let divider = self.color_from_palette("divider", "#363c46");
        let panel_strong = self.color_from_palette("panelStrong", "#2b2f36");
        let panel_tint = self.color_from_palette("panelTint", "#323846");
        let text_base = self.color_from_palette("textBase", "#cdd6dd");
        let text_muted = self.color_from_palette("textMuted", "#8f9aa6");
        let text_strong = self.color_from_palette("textStrong", "#eff4f8");
        let accent = self.color_from_palette("accent", "#78a7ff");
        let accent_strong = self.color_from_palette("accentStrong", "#4ea0ff");
        let success_text = self.color_from_palette("successText", "#6fdd8b");
        let warning_text = self.color_from_palette("warningText", "#e0b46a");
        let selection_bg = self.color_from_palette("selectionBg", "#31445b");
        let line_context = self.color_from_palette("lineContext", "#242a33");
        let line_context_alt = self.color_from_palette("lineContextAlt", "#262d36");
        let line_add = self.color_from_palette("lineAdd", "#1f2d24");
        let line_add_accent = self.color_from_palette("lineAddAccent", "#214d31");
        let line_del = self.color_from_palette("lineDel", "#2d2024");
        let line_del_accent = self.color_from_palette("lineDelAccent", "#56333a");
        let (selection_start, selection_end) = self.selected_bounds().unwrap_or((-1, -1));
        let family = self.monoFontFamily.clone();
        let hovered_row = self.hovered_row;
        let item = self.cpp_item_ptr();
        let rows_ptr = rows.as_ptr();
        let row_count = rows.len() as u32;
        let first_row_index = slot_row_start as u32;
        let lines_ptr = lines.as_ptr();
        let runs_ptr = runs.as_ptr();
        let bytes_ptr = bytes.as_ptr();

        let slot = &mut self.strip_slots[slot_index];
        if slot_image_width_px != image_width_px
            || slot_image_height_px != image_height_px
            || (slot_image_dpr - dpr).abs() > f64::EPSILON
        {
            Self::release_slot_texture(slot);
            slot.image = QImage::new(
                QSize {
                    width: image_width_px as u32,
                    height: image_height_px as u32,
                },
                ImageFormat::ARGB32_Premultiplied,
            );
            let image_ptr = &mut slot.image;
            cpp!(unsafe [image_ptr as "QImage*", dpr as "double"] {
                diffySetImageDevicePixelRatio(image_ptr, dpr);
            });
            slot.image_width_px = image_width_px;
            slot.image_height_px = image_height_px;
            slot.image_dpr = dpr;
        }
        let image_ptr = &mut slot.image;
        let raster_start = Instant::now();

        cpp!(unsafe [
            image_ptr as "QImage*",
            rows_ptr as "const DiffDisplayRow*",
            row_count as "quint32",
            first_row_index as "quint32",
            lines_ptr as "const DiffRenderLine*",
            runs_ptr as "const DiffStyleRun*",
            bytes_ptr as "const unsigned char*",
            split_mode as "bool",
            wrap_enabled as "bool",
            viewport_x as "quint32",
            viewport_y as "quint32",
            strip_top as "quint32",
            strip_height as "quint32",
            gutter_digits as "quint32",
            char_width as "double",
            line_height_px as "double",
            body_font_px as "double",
            unified_text_start_px as "double",
            unified_text_width_px as "double",
            split_side_width_px as "double",
            split_text_start_px as "double",
            split_text_width_px as "double",
            family as "QString",
            canvas as "QColor",
            divider as "QColor",
            panel_strong as "QColor",
            panel_tint as "QColor",
            text_base as "QColor",
            text_muted as "QColor",
            text_strong as "QColor",
            accent as "QColor",
            accent_strong as "QColor",
            success_text as "QColor",
            warning_text as "QColor",
            selection_bg as "QColor",
            line_context as "QColor",
            line_context_alt as "QColor",
            line_add as "QColor",
            line_add_accent as "QColor",
            line_del as "QColor",
            line_del_accent as "QColor",
            hovered_row as "int",
            selection_start as "int",
            selection_end as "int"
        ] {
            diffyRasterStrip(
                image_ptr,
                rows_ptr,
                row_count,
                first_row_index,
                lines_ptr,
                runs_ptr,
                bytes_ptr,
                split_mode,
                wrap_enabled,
                viewport_x,
                viewport_y,
                strip_top,
                strip_height,
                gutter_digits,
                char_width,
                line_height_px,
                body_font_px,
                unified_text_start_px,
                unified_text_width_px,
                split_side_width_px,
                split_text_start_px,
                split_text_width_px,
                family,
                canvas,
                divider,
                panel_strong,
                panel_tint,
                text_base,
                text_muted,
                text_strong,
                accent,
                accent_strong,
                success_text,
                warning_text,
                selection_bg,
                line_context,
                line_context_alt,
                line_add,
                line_add_accent,
                line_del,
                line_del_accent,
                hovered_row,
                selection_start,
                selection_end
            );
        });
        self.lastRasterTimeMs = elapsed_ms(raster_start);
        self.last_raster_time_ms_value = self.lastRasterTimeMs;

        let new_texture = {
            let image_ptr = &mut slot.image;
            cpp!(unsafe [item as "QQuickItem*", image_ptr as "QImage*"] -> *mut c_void as "QSGTexture*" {
                return diffyCreateTexture(item, image_ptr);
            })
        };

        if !slot.texture_raw.is_null() {
            self.stale_textures.push(slot.texture_raw);
        }
        slot.texture_raw = new_texture;
        slot.rendered_version = self.render_version;
        self.strip_reraster_count_value += 1;
    }

    fn prepare_visible_slots(&mut self) {
        self.strip_reuse_count_value = 0;
        self.strip_reraster_count_value = 0;
        self.active_slots.clear();

        let (start_strip, strip_count) = self.visible_strip_window();
        self.strip_count_value = strip_count as i32;
        if strip_count == 0 {
            return;
        }

        self.ensure_slot_pool(strip_count);
        for used in &mut self.slot_marks {
            *used = false;
        }

        for offset in 0..strip_count {
            let strip = self.strip_layouts_store[start_strip + offset];
            let slot_index = self.acquire_slot_index(strip.strip_id);
            let slot = &mut self.strip_slots[slot_index];
            if slot_needs_raster(slot, strip, self.render_version) {
                slot.rendered_version = 0;
            }
            slot.strip_id = strip.strip_id;
            slot.top_px = strip.top_px;
            slot.logical_height_px = strip.height_px;
            slot.row_start = strip.row_start;
            slot.row_end = strip.row_end;
            if slot.row_start >= slot.row_end || slot.logical_height_px == 0 {
                continue;
            }
            self.active_slots.push(slot_index);
        }

        for active_index in 0..self.active_slots.len() {
            let slot_index = self.active_slots[active_index];
            let strip = self.strip_layouts_store[start_strip + active_index];
            if slot_needs_raster(&self.strip_slots[slot_index], strip, self.render_version) {
                self.rasterize_slot(slot_index);
            }
        }
    }

    fn sync_perf_counters(&mut self, paint_ms: f64) {
        self.paint_count_value = self.paint_count_value.saturating_add(1);
        self.paint_count = self.paint_count_value;
        self.paintCountChanged();

        self.strip_count = self.strip_count_value;
        self.strip_reuse_count = self.strip_reuse_count_value;
        self.strip_reraster_count = self.strip_reraster_count_value;
        self.lastPaintTimeMs = paint_ms;
        self.last_paint_time_ms_value = paint_ms;
        self.perfStatsChanged();
    }

    pub fn get_content_height(&self) -> f64 {
        self.content_height
    }

    pub fn get_content_width(&self) -> f64 {
        self.content_width
    }

    pub fn get_paint_count(&self) -> i32 {
        self.paint_count_value
    }

    pub fn get_display_row_count(&self) -> i32 {
        i32::try_from(self.display_rows_store.len()).unwrap_or(i32::MAX)
    }

    pub fn get_strip_count(&self) -> i32 {
        self.strip_count_value
    }

    pub fn get_strip_reuse_count(&self) -> i32 {
        self.strip_reuse_count_value
    }

    pub fn get_strip_reraster_count(&self) -> i32 {
        self.strip_reraster_count_value
    }

    pub fn get_last_paint_time_ms(&self) -> f64 {
        self.last_paint_time_ms_value
    }

    pub fn get_last_raster_time_ms(&self) -> f64 {
        self.last_raster_time_ms_value
    }

    pub fn get_last_layout_time_ms(&self) -> f64 {
        self.last_layout_time_ms_value
    }

    pub fn set_render_key(&mut self, key: i64) {
        if self.render_key == key {
            return;
        }
        self.render_key = key;
        self.doc = clone_render_doc(key);
        self.hovered_row = -1;
        self.selection_anchor_row = -1;
        self.selection_cursor_row = -1;
        self.rebuild_display_rows();
        self.render_key_changed();
    }

    pub fn set_layout_mode(&mut self, mode: QString) {
        if self.layout_mode == mode {
            return;
        }
        self.layout_mode = mode;
        self.rebuild_display_rows();
        self.layout_mode_changed();
    }

    pub fn set_palette(&mut self, palette: QVariantMap) {
        self.palette = palette;
        self.palette_changed();
        self.invalidate_rendering();
    }

    pub fn set_mono_font_family(&mut self, family: QString) {
        if self.monoFontFamily == family {
            return;
        }
        self.monoFontFamily = family;
        self.refresh_font_metrics();
        self.mono_font_family_changed();
        self.rebuild_display_rows();
    }

    pub fn set_viewport_x(&mut self, value: f64) {
        if (self.viewport_x - value).abs() < f64::EPSILON {
            return;
        }
        self.viewport_x = value.max(0.0);
        self.viewport_x_changed();
        self.invalidate_rendering();
    }

    pub fn set_viewport_y(&mut self, value: f64) {
        if (self.viewport_y - value).abs() < f64::EPSILON {
            return;
        }
        self.viewport_y = value.max(0.0);
        self.viewport_y_changed();
        self.update_item();
    }

    pub fn set_viewport_height(&mut self, value: f64) {
        if (self.viewport_height - value).abs() < f64::EPSILON {
            return;
        }
        self.viewport_height = value.max(0.0);
        self.viewport_height_changed();
        self.update_item();
    }

    pub fn set_wrap_enabled(&mut self, value: bool) {
        if self.wrap_enabled == value {
            return;
        }
        self.wrap_enabled = value;
        self.wrap_enabled_changed();
        self.rebuild_display_rows();
    }

    pub fn set_wrap_column(&mut self, value: i32) {
        if self.wrap_column == value {
            return;
        }
        self.wrap_column = value.max(0);
        self.wrap_column_changed();
        self.rebuild_display_rows();
    }
}

fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

impl QQuickItem for DiffSurfaceItem {
    fn class_begin(&mut self) {
        self.set_flag(QQuickItemFlag::ItemHasContents);
        self.set_flag(QQuickItemFlag::ItemAcceptsInputMethod);
        self.set_flag(QQuickItemFlag::ItemIsFocusScope);
        self.set_accepted_mouse_buttons();
        self.set_accept_hover_events(true);
    }

    fn geometry_changed(&mut self, new_geometry: QRectF, old_geometry: QRectF) {
        self.viewport_height = new_geometry.height.max(0.0);
        self.viewport_height_changed();
        if (new_geometry.width - old_geometry.width).abs() > f64::EPSILON {
            self.rebuild_display_rows();
        } else {
            self.update_item();
        }
    }

    fn mouse_event(&mut self, event: QMouseEvent) -> bool {
        let row_index = self.row_index_at_y(event.position().y + self.viewport_y);
        match event.event_type() {
            QMouseEventType::MouseButtonPress => {
                self.force_focus();
                self.selection_anchor_row = row_index;
                self.selection_cursor_row = row_index;
                self.hovered_row = row_index;
                self.invalidate_rendering();
                true
            }
            QMouseEventType::MouseMove => {
                self.hovered_row = row_index;
                if self.selection_anchor_row >= 0 {
                    self.selection_cursor_row = row_index;
                }
                self.invalidate_rendering();
                true
            }
            QMouseEventType::MouseButtonRelease => {
                self.selection_cursor_row = row_index;
                self.invalidate_rendering();
                true
            }
            _ => false,
        }
    }

    fn release_resources(&mut self) {
        for slot in &mut self.strip_slots {
            Self::release_slot_texture(slot);
        }
        for texture in self.stale_textures.drain(..) {
            if texture.is_null() {
                continue;
            }
            cpp!(unsafe [texture as "QSGTexture*"] {
                diffyDeleteTexture(texture);
            });
        }
    }

    fn update_paint_node(&mut self, mut node: SGNode<ContainerNode>) -> SGNode<ContainerNode> {
        let paint_start = Instant::now();
        let raw = node.raw;
        let root = cpp!(unsafe [raw as "QSGNode*"] -> *mut c_void as "QSGNode*" {
            return diffyEnsureRoot(raw);
        });
        node.raw = root;

        self.prepare_visible_slots();

        for (child_index, &slot_index) in self.active_slots.iter().enumerate() {
            let slot = &self.strip_slots[slot_index];
            let rect = QRectF {
                x: 0.0,
                y: f64::from(slot.top_px) - self.viewport_y,
                width: self.bounding_width(),
                height: f64::from(slot.logical_height_px.max(1)),
            };
            let texture = slot.texture_raw;
            let child_index = child_index as i32;
            cpp!(unsafe [
                root as "QSGNode*",
                child_index as "int",
                rect as "QRectF",
                texture as "QSGTexture*"
            ] {
                diffySyncChild(root, child_index, rect, texture);
            });
        }

        let active_count = self.active_slots.len() as i32;
        cpp!(unsafe [root as "QSGNode*", active_count as "int"] {
            diffyTrimChildren(root, active_count);
        });

        for texture in self.stale_textures.drain(..) {
            if texture.is_null() {
                continue;
            }
            cpp!(unsafe [texture as "QSGTexture*"] {
                diffyDeleteTexture(texture);
            });
        }

        self.sync_perf_counters(elapsed_ms(paint_start));
        node
    }
}

#[cfg(test)]
mod tests {
    use super::{StripSlot, slot_needs_raster};
    use crate::app::surface::strip_layout::StripLayout;
    use std::ffi::c_void;

    #[test]
    fn reused_slot_with_new_strip_is_forced_to_rasterize() {
        let slot = StripSlot {
            strip_id: 0,
            top_px: 0,
            logical_height_px: 384,
            row_start: 0,
            row_end: 18,
            rendered_version: 7,
            texture_raw: 1usize as *mut c_void,
            ..StripSlot::default()
        };
        let next_strip = StripLayout {
            strip_id: 18,
            top_px: 384,
            height_px: 374,
            row_start: 18,
            row_end: 35,
        };

        assert!(slot_needs_raster(&slot, next_strip, 7));
    }

    #[test]
    fn matching_slot_with_live_texture_skips_rasterize() {
        let strip = StripLayout {
            strip_id: 18,
            top_px: 384,
            height_px: 374,
            row_start: 18,
            row_end: 35,
        };
        let slot = StripSlot {
            strip_id: strip.strip_id,
            top_px: strip.top_px,
            logical_height_px: strip.height_px,
            row_start: strip.row_start,
            row_end: strip.row_end,
            rendered_version: 7,
            texture_raw: 1usize as *mut c_void,
            ..StripSlot::default()
        };

        assert!(!slot_needs_raster(&slot, strip, 7));
    }
}
