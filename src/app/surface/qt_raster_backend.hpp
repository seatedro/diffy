#pragma once

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

QSGNode *diffyEnsureRoot(QSGNode *root);
void diffySyncChild(QSGNode *root, int index, QRectF rect, QSGTexture *texture);
void diffyTrimChildren(QSGNode *root, int active_count);
void diffyDeleteTexture(QSGTexture *texture);
QSGTexture *diffyCreateTexture(QQuickItem *item, QImage *image);
double diffyEffectiveDevicePixelRatio(QQuickItem *item);
void diffySetImageDevicePixelRatio(QImage *image, double dpr);

DiffFontMetrics diffyMeasureFontMetrics(QString family, int pixel_size);
double diffyMeasureTextWidth(QString family, int pixel_size, QString text);
quint16 diffyWrapLineCount(
    QString family,
    int pixel_size,
    QString text,
    double width,
    bool wrap_enabled
);

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
);
