#include "ui/DiffSurfaceItem.h"

#include <QClipboard>
#include <QColor>
#include <QFont>
#include <QFontMetricsF>
#include <QGuiApplication>
#include <QPainter>
#include <QtMath>

#include <algorithm>

namespace diffy {
namespace {

QFont monoFont(const QString& family, qreal pixelSize) {
  QFont font(family);
  font.setStyleHint(QFont::Monospace);
  font.setPixelSize(qRound(pixelSize));
  return font;
}

QVector<DiffSurfaceItem::TokenSpan> parseTokens(const QVariantList& tokenValues) {
  QVector<DiffSurfaceItem::TokenSpan> tokens;
  tokens.reserve(tokenValues.size());
  for (const QVariant& tokenValue : tokenValues) {
    const QVariantMap token = tokenValue.toMap();
    tokens.push_back(DiffSurfaceItem::TokenSpan{token.value("start").toInt(),
                                                token.value("length").toInt()});
  }
  return tokens;
}

}  // namespace

DiffSurfaceItem::DiffSurfaceItem(QQuickItem* parent) : QQuickPaintedItem(parent) {
  setOpaquePainting(false);
  setAcceptedMouseButtons(Qt::LeftButton);
  setAcceptHoverEvents(true);
  setFocus(true);
  connect(this, &QQuickItem::widthChanged, this, [this]() { update(); });
  connect(this, &QQuickItem::heightChanged, this, [this]() { update(); });
}

QVariantList DiffSurfaceItem::rowsModel() const {
  return rowsModel_;
}

void DiffSurfaceItem::setRowsModel(const QVariantList& rows) {
  if (rowsModel_ == rows) {
    return;
  }
  rowsModel_ = rows;
  rebuildRows();
  emit rowsModelChanged();
}

QString DiffSurfaceItem::layoutMode() const {
  return layoutMode_;
}

void DiffSurfaceItem::setLayoutMode(const QString& mode) {
  if (layoutMode_ == mode) {
    return;
  }
  layoutMode_ = mode;
  rebuildDisplayRows();
  emit layoutModeChanged();
}

QVariantMap DiffSurfaceItem::palette() const {
  return palette_;
}

void DiffSurfaceItem::setPalette(const QVariantMap& palette) {
  if (palette_ == palette) {
    return;
  }
  palette_ = palette;
  update();
  emit paletteChanged();
}

QString DiffSurfaceItem::monoFontFamily() const {
  return monoFontFamily_;
}

void DiffSurfaceItem::setMonoFontFamily(const QString& family) {
  if (monoFontFamily_ == family) {
    return;
  }
  monoFontFamily_ = family;
  recalculateMetrics();
  emit monoFontFamilyChanged();
}

qreal DiffSurfaceItem::contentHeight() const {
  return contentHeight_;
}

qreal DiffSurfaceItem::contentWidth() const {
  return contentWidth_;
}

qreal DiffSurfaceItem::viewportY() const {
  return viewportY_;
}

void DiffSurfaceItem::setViewportY(qreal value) {
  if (qFuzzyCompare(viewportY_, value)) {
    return;
  }
  viewportY_ = value;
  const int nextFirst = rowIndexAtY(std::max<qreal>(0.0, viewportY_ - hunkHeight_));
  const int nextLast = rowIndexAtY(viewportY_ + viewportHeight_ + hunkHeight_);
  const int nextSticky = stickyRowIndexAtY(viewportY_);
  if (nextFirst != firstVisibleRow_ || nextLast != lastVisibleRow_ || nextSticky != stickyVisibleRow_) {
    firstVisibleRow_ = nextFirst;
    lastVisibleRow_ = nextLast;
    stickyVisibleRow_ = nextSticky;
    update();
  }
  emit viewportYChanged();
}

qreal DiffSurfaceItem::viewportHeight() const {
  return viewportHeight_;
}

void DiffSurfaceItem::setViewportHeight(qreal value) {
  if (qFuzzyCompare(viewportHeight_, value)) {
    return;
  }
  viewportHeight_ = value;
  firstVisibleRow_ = rowIndexAtY(std::max<qreal>(0.0, viewportY_ - hunkHeight_));
  lastVisibleRow_ = rowIndexAtY(viewportY_ + viewportHeight_ + hunkHeight_);
  stickyVisibleRow_ = stickyRowIndexAtY(viewportY_);
  update();
  emit viewportHeightChanged();
}

int DiffSurfaceItem::paintCount() const {
  return paintCount_;
}

int DiffSurfaceItem::displayRowCount() const {
  return displayRows_.size();
}

void DiffSurfaceItem::paint(QPainter* painter) {
  ++paintCount_;

  const QRectF exposedRect = painter->clipBoundingRect();
  painter->fillRect(exposedRect, paletteColor("canvas", QColor("#282c33")));

  if (displayRows_.isEmpty()) {
    return;
  }

  painter->setRenderHint(QPainter::TextAntialiasing, true);
  painter->setRenderHint(QPainter::Antialiasing, false);

  const qreal visibleTop = viewportHeight_ > 0 ? std::max<qreal>(0.0, viewportY_ - hunkHeight_) : exposedRect.top();
  const qreal visibleBottom = viewportHeight_ > 0 ? viewportY_ + viewportHeight_ + hunkHeight_ : exposedRect.bottom();

  int firstRow = rowIndexAtY(visibleTop);
  int lastRow = rowIndexAtY(visibleBottom);
  if (firstRow < 0) {
    firstRow = 0;
  }
  if (lastRow < 0) {
    lastRow = displayRows_.size() - 1;
  }

  for (int rowIndex = firstRow; rowIndex <= lastRow && rowIndex < displayRows_.size(); ++rowIndex) {
    const Row& row = displayRows_.at(rowIndex);
    const QRectF rowRect(0, row.top, width(), row.height);
    const bool selected = rowSelected(rowIndex);
    const bool hovered = hoveredRow_ == rowIndex;

    if (row.rowType == "hunk") {
      drawHunkRow(painter, rowRect, row);
      if (selected) {
        QColor selection = paletteColor("selectionBg", QColor("#3c3836"));
        selection.setAlpha(110);
        painter->fillRect(rowRect, selection);
      } else if (hovered) {
        QColor hover = paletteColor("panelTint", QColor("#504945"));
        hover.setAlpha(90);
        painter->fillRect(rowRect, hover);
      }
      continue;
    }

    if (layoutMode_ == "split") {
      drawSplitRow(painter, rowRect, row, selected || hovered);
    } else {
      drawUnifiedRow(painter, rowRect, row, selected || hovered);
    }
  }

  if (viewportHeight_ > 0) {
    int stickyIndex = -1;
    for (int rowIndex = 0; rowIndex < displayRows_.size(); ++rowIndex) {
      const Row& row = displayRows_.at(rowIndex);
      if (row.rowType == "hunk" && row.top <= viewportY_) {
        stickyIndex = rowIndex;
      }
      if (row.top > viewportY_) {
        break;
      }
    }

    if (stickyIndex >= 0) {
      qreal stickyY = viewportY_;
      for (int nextIndex = stickyIndex + 1; nextIndex < displayRows_.size(); ++nextIndex) {
        const Row& nextRow = displayRows_.at(nextIndex);
        if (nextRow.rowType == "hunk") {
          stickyY = std::min(stickyY, nextRow.top - hunkHeight_);
          break;
        }
      }

      painter->save();
      QColor shadow = paletteColor("canvas", QColor("#282828"));
      shadow.setAlpha(210);
      painter->fillRect(QRectF(0, stickyY, width(), hunkHeight_), shadow);
      drawHunkRow(painter, QRectF(0, stickyY, width(), hunkHeight_), displayRows_.at(stickyIndex));
      painter->restore();
    }
  }
}

void DiffSurfaceItem::rebuildRows() {
  sourceRows_.clear();
  rowOffsets_.clear();
  textRope_.clear();
  textCache_.clear();

  const QFontMetricsF metrics(monoFont(monoFontFamily_, 12));
  lineHeight_ = metrics.height();
  rowHeight_ = qCeil(lineHeight_ + 6.0);
  hunkHeight_ = 24.0;

  for (const QVariant& rowValue : rowsModel_) {
    const QVariantMap rowMap = rowValue.toMap();
    Row row;
    row.rowType = rowMap.value("rowType").toString();
    row.header = rowMap.value("header").toString();
    row.kind = rowMap.value("kind").toString();
    row.oldLine = rowMap.contains("oldLine") ? rowMap.value("oldLine").toInt() : -1;
    row.newLine = rowMap.contains("newLine") ? rowMap.value("newLine").toInt() : -1;
    row.textRange = textRope_.append(rowMap.value("text").toString().toUtf8().toStdString());
    row.tokens = parseTokens(rowMap.value("tokens").toList());
    sourceRows_.push_back(row);
  }

  rebuildDisplayRows();
}

void DiffSurfaceItem::rebuildDisplayRows() {
  displayRows_.clear();
  rowOffsets_.clear();

  const QFontMetricsF metrics(monoFont(monoFontFamily_, 12));
  lineHeight_ = metrics.height();
  rowHeight_ = qCeil(lineHeight_ + 6.0);
  hunkHeight_ = 24.0;

  int maxLineNumber = 0;
  qreal top = 0;
  maxTextWidth_ = 0;

  auto appendRow = [&](Row row) {
    row.top = top;
    row.height = row.rowType == "hunk" ? hunkHeight_ : rowHeight_;
    rowOffsets_.push_back(top);
    top += row.height;
    maxLineNumber = std::max(maxLineNumber, std::max(row.oldLine, row.newLine));
    maxLineNumber = std::max(maxLineNumber, std::max(row.leftLine, row.rightLine));
    maxTextWidth_ = std::max(maxTextWidth_, metrics.horizontalAdvance(textForRange(row.textRange)));
    maxTextWidth_ = std::max(maxTextWidth_, metrics.horizontalAdvance(textForRange(row.leftTextRange)));
    maxTextWidth_ = std::max(maxTextWidth_, metrics.horizontalAdvance(textForRange(row.rightTextRange)));
    displayRows_.push_back(row);
  };

  if (layoutMode_ != "split") {
    for (const Row& sourceRow : sourceRows_) {
      appendRow(sourceRow);
    }
  } else {
    for (int index = 0; index < sourceRows_.size(); ++index) {
      const Row& sourceRow = sourceRows_.at(index);
      if (sourceRow.rowType == "hunk") {
        appendRow(sourceRow);
        continue;
      }

      if (sourceRow.kind == "ctx") {
        Row row = sourceRow;
        row.leftKind = "ctx";
        row.rightKind = "ctx";
        row.leftLine = sourceRow.oldLine;
        row.rightLine = sourceRow.newLine;
        row.leftTextRange = sourceRow.textRange;
        row.rightTextRange = sourceRow.textRange;
        row.leftTokens = sourceRow.tokens;
        row.rightTokens = sourceRow.tokens;
        appendRow(row);
        continue;
      }

      if (sourceRow.kind == "add" || sourceRow.kind == "del") {
        QVector<Row> deletions;
        QVector<Row> additions;

        while (index < sourceRows_.size()) {
          const Row& blockRow = sourceRows_.at(index);
          if (blockRow.rowType != "line" || blockRow.kind == "ctx") {
            --index;
            break;
          }

          if (blockRow.kind == "del") {
            deletions.push_back(blockRow);
          } else if (blockRow.kind == "add") {
            additions.push_back(blockRow);
          }
          ++index;
        }

        const int rowCount = std::max(deletions.size(), additions.size());
        for (int rowIndex = 0; rowIndex < rowCount; ++rowIndex) {
          Row row;
          row.rowType = "line";
          row.kind = "change";

          if (rowIndex < deletions.size()) {
            const Row& left = deletions.at(rowIndex);
            row.leftKind = "del";
            row.leftLine = left.oldLine;
            row.leftTextRange = left.textRange;
            row.leftTokens = left.tokens;
            row.oldLine = left.oldLine;
          } else {
            row.leftKind = "spacer";
          }

          if (rowIndex < additions.size()) {
            const Row& right = additions.at(rowIndex);
            row.rightKind = "add";
            row.rightLine = right.newLine;
            row.rightTextRange = right.textRange;
            row.rightTokens = right.tokens;
            row.newLine = right.newLine;
          } else {
            row.rightKind = "spacer";
          }

          appendRow(row);
        }
      }
    }
  }

  contentHeight_ = top;
  lineNumberDigits_ = std::max(3, static_cast<int>(QString::number(std::max(0, maxLineNumber)).size()));
  emit displayRowCountChanged();

  recalculateMetrics();
}

void DiffSurfaceItem::recalculateMetrics() {
  qreal newContentWidth = 0;
  if (layoutMode_ == "split") {
    const qreal sideBase = 72.0;
    newContentWidth = maxTextWidth_ * 2.0 + sideBase * 2.0 + 1.0;
  } else {
    newContentWidth = unifiedGutterWidth() + maxTextWidth_ + 24.0;
  }

  if (!qFuzzyCompare(contentWidth_, newContentWidth)) {
    contentWidth_ = newContentWidth;
    emit contentWidthChanged();
  }

  emit contentHeightChanged();
  update();
}

int DiffSurfaceItem::rowIndexAtY(qreal y) const {
  if (displayRows_.isEmpty()) {
    return -1;
  }

  const auto it = std::upper_bound(rowOffsets_.cbegin(), rowOffsets_.cend(), y);
  if (it == rowOffsets_.cbegin()) {
    return 0;
  }
  return std::clamp(static_cast<int>(std::distance(rowOffsets_.cbegin(), it) - 1), 0,
                    static_cast<int>(displayRows_.size() - 1));
}

int DiffSurfaceItem::stickyRowIndexAtY(qreal y) const {
  int stickyIndex = -1;
  for (int rowIndex = 0; rowIndex < displayRows_.size(); ++rowIndex) {
    const Row& row = displayRows_.at(rowIndex);
    if (row.rowType == "hunk" && row.top <= y) {
      stickyIndex = rowIndex;
    }
    if (row.top > y) {
      break;
    }
  }
  return stickyIndex;
}

bool DiffSurfaceItem::rowSelected(int rowIndex) const {
  if (selectionAnchorRow_ < 0 || selectionCursorRow_ < 0) {
    return false;
  }
  const int start = std::min(selectionAnchorRow_, selectionCursorRow_);
  const int end = std::max(selectionAnchorRow_, selectionCursorRow_);
  return rowIndex >= start && rowIndex <= end;
}

QString DiffSurfaceItem::textForRange(const TextRange& range) const {
  const quint64 key = (static_cast<quint64>(range.start) << 32) | static_cast<quint64>(range.length);
  if (const auto it = textCache_.constFind(key); it != textCache_.constEnd()) {
    return it.value();
  }
  const QString text = QString::fromUtf8(textRope_.slice(range));
  textCache_.insert(key, text);
  return text;
}

QColor DiffSurfaceItem::paletteColor(const QString& key, const QColor& fallback) const {
  const QVariant value = palette_.value(key);
  if (!value.isValid()) {
    return fallback;
  }
  const QColor color = value.value<QColor>();
  return color.isValid() ? color : fallback;
}

qreal DiffSurfaceItem::digitWidth() const {
  const QFontMetricsF metrics(monoFont(monoFontFamily_, 10));
  return metrics.horizontalAdvance(QLatin1Char('9'));
}

qreal DiffSurfaceItem::unifiedGutterWidth() const {
  return 12.0 + 12.0 + digitWidth() * (lineNumberDigits_ * 2 + 2) + 24.0;
}

void DiffSurfaceItem::drawHunkRow(QPainter* painter, const QRectF& rowRect, const Row& row) const {
  painter->fillRect(rowRect, paletteColor("panelStrong", QColor("#3b414d")));
  painter->fillRect(QRectF(rowRect.left(), rowRect.bottom() - 1.0, rowRect.width(), 1.0),
                    paletteColor("divider", QColor("#363c46")));

  painter->setFont(monoFont(monoFontFamily_, 10));
  painter->setPen(paletteColor("textMuted", QColor("#a9afbc")));
  painter->drawText(QRectF(rowRect.left() + 10.0, rowRect.top(), rowRect.width() - 20.0, rowRect.height()),
                    Qt::AlignVCenter | Qt::AlignLeft, row.header);
}

void DiffSurfaceItem::drawUnifiedRow(QPainter* painter, const QRectF& rowRect, const Row& row, bool selected) const {
  QColor background = paletteColor("lineContext", QColor("#282c33"));
  if (row.kind == "add") {
    background = paletteColor("lineAdd", QColor("#1f2d24"));
  } else if (row.kind == "del") {
    background = paletteColor("lineDel", QColor("#2d2024"));
  }

  painter->fillRect(rowRect, background);
  if (selected) {
    QColor selection = paletteColor("selectionBg", QColor("#3c3836"));
    selection.setAlpha(90);
    painter->fillRect(rowRect, selection);
  }

  const qreal gutterWidth = unifiedGutterWidth();
  const QRectF gutterRect(rowRect.left(), rowRect.top(), gutterWidth, rowRect.height());
  painter->fillRect(gutterRect, paletteColor("panelTint", QColor("#353b45")));
  painter->fillRect(QRectF(gutterRect.right(), rowRect.top(), 1.0, rowRect.height()),
                    paletteColor("divider", QColor("#363c46")));

  if (row.kind == "add" || row.kind == "del") {
    const QColor marker = row.kind == "add" ? paletteColor("successText", QColor("#a1c181"))
                                            : paletteColor("dangerText", QColor("#d07277"));
    painter->fillRect(QRectF(rowRect.left(), rowRect.top(), 2.0, rowRect.height()), marker);
  }

  painter->setFont(monoFont(monoFontFamily_, 10));
  painter->setPen(paletteColor("textFaint", QColor("#878a98")));

  const qreal indicatorX = rowRect.left() + 6.0;
  painter->setPen(row.kind == "add" ? paletteColor("successText", QColor("#a1c181"))
                                    : row.kind == "del" ? paletteColor("dangerText", QColor("#d07277"))
                                                        : paletteColor("textFaint", QColor("#878a98")));
  painter->drawText(QRectF(indicatorX, rowRect.top(), 10.0, rowRect.height()), Qt::AlignVCenter, row.kind == "add" ? "+"
                                                                                                                     : row.kind == "del" ? "-"
                                                                                                                                         : " ");

  painter->setPen(paletteColor("textFaint", QColor("#878a98")));
  const qreal numberWidth = digitWidth() * lineNumberDigits_;
  painter->drawText(QRectF(rowRect.left() + 22.0, rowRect.top(), numberWidth, rowRect.height()),
                    Qt::AlignRight | Qt::AlignVCenter,
                    row.oldLine > 0 ? QString::number(row.oldLine) : QString());
  painter->drawText(QRectF(rowRect.left() + 34.0 + numberWidth, rowRect.top(), numberWidth, rowRect.height()),
                    Qt::AlignRight | Qt::AlignVCenter,
                    row.newLine > 0 ? QString::number(row.newLine) : QString());

  const QFont textFont = monoFont(monoFontFamily_, 12);
  const QFontMetricsF textMetrics(textFont);
  painter->setFont(textFont);
  const qreal baselineY = rowRect.top() + (rowRect.height() - textMetrics.height()) / 2.0 + textMetrics.ascent();
  const QRectF textClip(rowRect.left() + gutterWidth + 8.0, rowRect.top(), rowRect.width() - gutterWidth - 12.0, rowRect.height());
  const QColor textColor = paletteColor("textBase", QColor("#c8ccd4"));
  const QColor tokenBg = row.kind == "add" ? paletteColor("successBorder", QColor("#38482f"))
                                           : row.kind == "del" ? paletteColor("dangerBorder", QColor("#4c2b2c"))
                                                               : paletteColor("accentSoft", QColor("#293b5b"));
  drawTextRun(painter, QPointF(textClip.left(), baselineY), textClip, textForRange(row.textRange), row.tokens,
              textColor, tokenBg);
}

void DiffSurfaceItem::drawSplitRow(QPainter* painter, const QRectF& rowRect, const Row& row, bool selected) const {
  const QRectF leftRect(rowRect.left(), rowRect.top(), rowRect.width() / 2.0, rowRect.height());
  const QRectF rightRect(leftRect.right(), rowRect.top(), rowRect.width() - leftRect.width(), rowRect.height());
  const qreal sideGutterWidth = 58.0;

  const QColor leftBg = row.leftKind == "del" ? paletteColor("lineDelAccent", QColor("#35262b"))
                                          : paletteColor("lineContext", QColor("#282c33"));
  const QColor rightBg = row.rightKind == "add" ? paletteColor("lineAddAccent", QColor("#22332a"))
                                           : paletteColor("lineContext", QColor("#282c33"));
  painter->fillRect(leftRect, leftBg);
  painter->fillRect(rightRect, rightBg);
  if (selected) {
    QColor selection = paletteColor("selectionBg", QColor("#3c3836"));
    selection.setAlpha(90);
    painter->fillRect(leftRect, selection);
    painter->fillRect(rightRect, selection);
  }
  painter->fillRect(QRectF(leftRect.left(), rowRect.top(), sideGutterWidth, rowRect.height()),
                    paletteColor("panelTint", QColor("#504945")));
  painter->fillRect(QRectF(rightRect.left(), rowRect.top(), sideGutterWidth, rowRect.height()),
                    paletteColor("panelTint", QColor("#504945")));
  painter->fillRect(QRectF(leftRect.left() + sideGutterWidth, rowRect.top(), 1.0, rowRect.height()),
                    paletteColor("divider", QColor("#504945")));
  painter->fillRect(QRectF(rightRect.left() + sideGutterWidth, rowRect.top(), 1.0, rowRect.height()),
                    paletteColor("divider", QColor("#504945")));
  painter->fillRect(QRectF(leftRect.right(), rowRect.top(), 1.0, rowRect.height()),
                    paletteColor("divider", QColor("#363c46")));

  if (row.leftKind == "del") {
    painter->fillRect(QRectF(leftRect.left(), rowRect.top(), 2.0, rowRect.height()),
                      paletteColor("dangerText", QColor("#d07277")));
  }
  if (row.rightKind == "add") {
    painter->fillRect(QRectF(rightRect.left(), rowRect.top(), 2.0, rowRect.height()),
                      paletteColor("successText", QColor("#a1c181")));
  }

  painter->setFont(monoFont(monoFontFamily_, 10));
  painter->setPen(paletteColor("textFaint", QColor("#878a98")));
  painter->drawText(QRectF(leftRect.left() + 6.0, leftRect.top(), 10.0, leftRect.height()), Qt::AlignVCenter,
                    row.leftKind == "del" ? "-" : row.leftKind == "ctx" ? " " : " ");
  painter->drawText(QRectF(rightRect.left() + 6.0, rightRect.top(), 10.0, rightRect.height()), Qt::AlignVCenter,
                    row.rightKind == "add" ? "+" : row.rightKind == "ctx" ? " " : " ");
  painter->drawText(QRectF(leftRect.left() + 8.0, leftRect.top(), 34.0, leftRect.height()),
                    Qt::AlignRight | Qt::AlignVCenter,
                    row.leftLine > 0 ? QString::number(row.leftLine) : QString());
  painter->drawText(QRectF(rightRect.left() + 8.0, rightRect.top(), 34.0, rightRect.height()),
                    Qt::AlignRight | Qt::AlignVCenter,
                    row.rightLine > 0 ? QString::number(row.rightLine) : QString());

  const QFont textFont = monoFont(monoFontFamily_, 12);
  const QFontMetricsF textMetrics(textFont);
  painter->setFont(textFont);
  const qreal baselineY = rowRect.top() + (rowRect.height() - textMetrics.height()) / 2.0 + textMetrics.ascent();
  const QRectF leftTextClip(leftRect.left() + sideGutterWidth + 8.0, leftRect.top(),
                            leftRect.width() - sideGutterWidth - 12.0, leftRect.height());
  const QRectF rightTextClip(rightRect.left() + sideGutterWidth + 8.0, rightRect.top(),
                             rightRect.width() - sideGutterWidth - 12.0, rightRect.height());

  if (row.leftKind != "spacer") {
    drawTextRun(painter, QPointF(leftTextClip.left(), baselineY), leftTextClip,
                textForRange(row.leftTextRange), row.leftTokens,
                paletteColor("textBase", QColor("#c8ccd4")),
                paletteColor("dangerBorder", QColor("#4c2b2c")));
  }

  if (row.rightKind != "spacer") {
    drawTextRun(painter, QPointF(rightTextClip.left(), baselineY), rightTextClip,
                textForRange(row.rightTextRange), row.rightTokens,
                paletteColor("textBase", QColor("#c8ccd4")),
                paletteColor("successBorder", QColor("#38482f")));
  }
}

void DiffSurfaceItem::drawTextRun(QPainter* painter,
                                  const QPointF& baseline,
                                  const QRectF& clipRect,
                                  const QString& text,
                                  const QVector<TokenSpan>& tokens,
                                  const QColor& textColor,
                                  const QColor& tokenBackground) const {
  painter->save();
  painter->setClipRect(clipRect);

  const QFont textFont = monoFont(monoFontFamily_, 12);
  const QFontMetricsF metrics(textFont);
  painter->setFont(textFont);

  if (!tokens.isEmpty() && !text.isEmpty()) {
    QVector<TokenSpan> sortedTokens = tokens;
    std::sort(sortedTokens.begin(), sortedTokens.end(), [](const TokenSpan& lhs, const TokenSpan& rhs) {
      return lhs.start < rhs.start;
    });

    for (const TokenSpan& token : sortedTokens) {
      const int start = std::max(0, token.start);
      const int end = std::min(static_cast<int>(text.size()), token.start + token.length);
      if (end <= start) {
        continue;
      }

      const qreal startX = baseline.x() + metrics.horizontalAdvance(text.left(start));
      const qreal tokenWidth = metrics.horizontalAdvance(text.mid(start, end - start));
      const QRectF tokenRect(startX - 1.0, baseline.y() - metrics.ascent() - 1.0, tokenWidth + 2.0, metrics.height() + 2.0);
      painter->fillRect(tokenRect, tokenBackground);
    }
  }

  painter->setPen(textColor);
  painter->drawText(baseline, text);
  painter->restore();
}

QString DiffSurfaceItem::selectedText() const {
  if (selectionAnchorRow_ < 0 || selectionCursorRow_ < 0 || displayRows_.isEmpty()) {
    return {};
  }

  QStringList parts;
  const int start = std::min(selectionAnchorRow_, selectionCursorRow_);
  const int end = std::max(selectionAnchorRow_, selectionCursorRow_);
  for (int rowIndex = start; rowIndex <= end && rowIndex < displayRows_.size(); ++rowIndex) {
    const Row& row = displayRows_.at(rowIndex);
    if (row.rowType == "hunk") {
      parts.push_back(row.header);
      continue;
    }

    if (layoutMode_ == "split") {
      if (row.leftKind == "ctx" && row.rightKind == "ctx") {
        parts.push_back(" " + textForRange(row.leftTextRange));
      } else {
        if (row.leftKind != "spacer") {
          parts.push_back("-" + textForRange(row.leftTextRange));
        }
        if (row.rightKind != "spacer") {
          parts.push_back("+" + textForRange(row.rightTextRange));
        }
      }
    } else {
      const QString prefix = row.kind == "add" ? "+" : row.kind == "del" ? "-" : " ";
      parts.push_back(prefix + textForRange(row.textRange));
    }
  }

  return parts.join('\n');
}

void DiffSurfaceItem::mousePressEvent(QMouseEvent* event) {
  forceActiveFocus(Qt::MouseFocusReason);
  const int rowIndex = rowIndexAtY(event->position().y());
  selectionAnchorRow_ = rowIndex;
  selectionCursorRow_ = rowIndex;
  update();
  QQuickPaintedItem::mousePressEvent(event);
}

void DiffSurfaceItem::mouseMoveEvent(QMouseEvent* event) {
  if (selectionAnchorRow_ >= 0) {
    selectionCursorRow_ = rowIndexAtY(event->position().y());
    update();
  }
  QQuickPaintedItem::mouseMoveEvent(event);
}

void DiffSurfaceItem::mouseReleaseEvent(QMouseEvent* event) {
  if (selectionAnchorRow_ >= 0) {
    selectionCursorRow_ = rowIndexAtY(event->position().y());
    update();
  }
  QQuickPaintedItem::mouseReleaseEvent(event);
}

void DiffSurfaceItem::keyPressEvent(QKeyEvent* event) {
  if (event->matches(QKeySequence::Copy)) {
    const QString text = selectedText();
    if (!text.isEmpty()) {
      if (QClipboard* clipboard = QGuiApplication::clipboard()) {
        clipboard->setText(text);
      }
      event->accept();
      return;
    }
  }
  if (event->matches(QKeySequence::SelectAll)) {
    if (!displayRows_.isEmpty()) {
      selectionAnchorRow_ = 0;
      selectionCursorRow_ = displayRows_.size() - 1;
      update();
    }
    event->accept();
    return;
  }
  QQuickPaintedItem::keyPressEvent(event);
}

void DiffSurfaceItem::hoverMoveEvent(QHoverEvent* event) {
  hoveredRow_ = rowIndexAtY(event->position().y());
  update();
  QQuickPaintedItem::hoverMoveEvent(event);
}

void DiffSurfaceItem::hoverLeaveEvent(QHoverEvent* event) {
  hoveredRow_ = -1;
  update();
  QQuickPaintedItem::hoverLeaveEvent(event);
}

}  // namespace diffy
