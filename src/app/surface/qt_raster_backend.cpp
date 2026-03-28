#include "app/surface/qt_raster_backend.hpp"

class DiffStripNode : public QSGSimpleTextureNode {
public:
    DiffStripNode() {
        setOwnsTexture(false);
    }
};

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
    case 1: return accent_strong;
    case 2: return success_text;
    case 3: return text_muted;
    case 4: return warning_text;
    case 5: return accent;
    case 6: return text_strong;
    case 7: return accent_strong;
    case 8: return text_muted;
    case 9: return accent;
    case 10: return warning_text;
    case 11: return accent_strong;
    case 12: return accent;
    case 13: return success_text;
    case 14: return accent;
    case 15: return accent;
    case 16: return warning_text;
    case 17: return warning_text;
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

QSGNode *diffyEnsureRoot(QSGNode *root) {
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

void diffyTrimChildren(QSGNode *root, int active_count) {
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

void diffySyncChild(QSGNode *root, int index, QRectF rect, QSGTexture *texture) {
    auto *node = diffyEnsureChild(root, index);
    if (!node) {
        return;
    }
    node->setTexture(texture);
    node->setRect(rect);
}

void diffyDeleteTexture(QSGTexture *texture) {
    delete texture;
}

QSGTexture *diffyCreateTexture(QQuickItem *item, QImage *image) {
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

double diffyEffectiveDevicePixelRatio(QQuickItem *item) {
    if (!item) {
        return 1.0;
    }
    auto *window = item->window();
    if (!window) {
        return 1.0;
    }
    return window->effectiveDevicePixelRatio();
}

void diffySetImageDevicePixelRatio(QImage *image, double dpr) {
    if (!image) {
        return;
    }
    image->setDevicePixelRatio(dpr > 0.0 ? dpr : 1.0);
}

DiffFontMetrics diffyMeasureFontMetrics(QString family, int pixel_size) {
    QFont font(family);
    font.setStyleHint(QFont::TypeWriter);
    font.setFixedPitch(true);
    font.setPixelSize(pixel_size);
    QFontMetricsF metrics(font);
    return DiffFontMetrics{
        metrics.horizontalAdvance(QStringLiteral("M")),
        metrics.height(),
        metrics.ascent(),
    };
}

double diffyMeasureTextWidth(QString family, int pixel_size, QString text) {
    if (text.isEmpty()) {
        return 0.0;
    }
    QFont font(family);
    font.setStyleHint(QFont::TypeWriter);
    font.setFixedPitch(true);
    font.setPixelSize(pixel_size);
    QFontMetricsF metrics(font);
    return metrics.horizontalAdvance(text);
}

quint16 diffyWrapLineCount(
    QString family,
    int pixel_size,
    QString text,
    double width,
    bool wrap_enabled
) {
    if (text.isEmpty() || !wrap_enabled || width <= 1.0) {
        return 1;
    }

    QFont font(family);
    font.setStyleHint(QFont::TypeWriter);
    font.setFixedPitch(true);
    font.setPixelSize(pixel_size);

    QTextOption option;
    option.setWrapMode(QTextOption::WrapAnywhere);

    QTextLayout layout(text, font);
    layout.setTextOption(option);
    layout.beginLayout();

    quint16 count = 0;
    while (true) {
        QTextLine text_line = layout.createLine();
        if (!text_line.isValid()) {
            break;
        }
        text_line.setLineWidth(width);
        ++count;
    }

    layout.endLayout();
    return count == 0 ? 1 : count;
}

void diffyRasterStrip(
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
    double body_row_height_px,
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
                if (
                    line.kind == 5 &&
                    line.left_text.start != quint32(-1) &&
                    line.right_text.start != quint32(-1)
                ) {
                    const qreal left_height = qreal(row.wrap_left) * body_row_height_px;
                    const qreal right_height = qreal(row.wrap_right) * body_row_height_px;
                    const qreal total_height = left_height + right_height;
                    const qreal old_height = left_height > 0.0 ? left_height : total_height * 0.5;
                    const qreal new_top = top + old_height;
                    const qreal new_height = right_height > 0.0 ? right_height : (height - old_height);

                    QColor old_bg = selected || hovered ? selection_bg : line_del;
                    QColor new_bg = selected || hovered ? selection_bg : line_add;
                    painter.fillRect(QRectF(0.0, top, image->width(), old_height), old_bg);
                    painter.fillRect(QRectF(0.0, new_top, image->width(), new_height), new_bg);

                    if (line.old_line_no != quint32(-1)) {
                        painter.drawText(
                            QRectF(0.0, top, number_width, old_height),
                            Qt::AlignRight | Qt::AlignVCenter,
                            QString::number(int(line.old_line_no))
                        );
                    }
                    if (line.new_line_no != quint32(-1)) {
                        painter.drawText(
                            QRectF(number_width, new_top, number_width, new_height),
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
                        line_del_accent,
                        wrap_enabled
                    );

                    diffyDrawStyledBlock(
                        painter,
                        text_bytes,
                        line,
                        false,
                        runs,
                        unified_text_start_px - qreal(viewport_x),
                        new_top + 2.0,
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
                        line_add_accent,
                        wrap_enabled
                    );
                } else {
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
    }

    painter.end();
    (void)viewport_y;
    (void)strip_height;
}
