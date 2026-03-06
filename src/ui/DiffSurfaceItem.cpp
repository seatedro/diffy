#include "ui/DiffSurfaceItem.h"

#include <QClipboard>
#include <QColor>
#include <QFont>
#include <QFontMetricsF>
#include <QGuiApplication>
#include <QPainter>
#include <QtMath>

#include <algorithm>

#include "core/SyntaxTypes.h"

namespace diffy {
namespace {

QColor syntaxForeground(SyntaxTokenKind kind) {
  switch (kind) {
    case SyntaxTokenKind::Keyword:
      return QColor("#fb4934");
    case SyntaxTokenKind::String:
      return QColor("#b8bb26");
    case SyntaxTokenKind::Comment:
      return QColor("#928374");
    case SyntaxTokenKind::Type:
      return QColor("#fabd2f");
    case SyntaxTokenKind::Function:
      return QColor("#b8bb26");
    case SyntaxTokenKind::Variable:
      return QColor("#83a598");
    case SyntaxTokenKind::Number:
      return QColor("#d3869b");
    case SyntaxTokenKind::Operator:
      return QColor("#fe8019");
    case SyntaxTokenKind::Punctuation:
      return QColor("#a89984");
    case SyntaxTokenKind::Property:
      return QColor("#83a598");
    case SyntaxTokenKind::Attribute:
      return QColor("#8ec07c");
    case SyntaxTokenKind::Namespace:
      return QColor("#fabd2f");
    case SyntaxTokenKind::Constant:
      return QColor("#d3869b");
    case SyntaxTokenKind::Label:
      return QColor("#fe8019");
    case SyntaxTokenKind::Embedded:
      return QColor("#8ec07c");
    default:
      return {};
  }
}

QFont monoFont(const QString& family, qreal pixelSize) {
  QFont font(family);
  font.setStyleHint(QFont::Monospace);
  font.setPixelSize(qRound(pixelSize));
  return font;
}

std::vector<DiffTokenSpan> parseTokens(const QVariantList& tokenValues) {
  std::vector<DiffTokenSpan> tokens;
  tokens.reserve(tokenValues.size());
  for (const QVariant& tokenValue : tokenValues) {
    const QVariantMap token = tokenValue.toMap();
    tokens.push_back(DiffTokenSpan{token.value("start").toInt(), token.value("length").toInt(),
                                   SyntaxTokenKind::None});
  }
  return tokens;
}

std::vector<DiffTokenSpan> parseTokens(const std::vector<TokenSpan>& tokenValues) {
  std::vector<DiffTokenSpan> tokens;
  tokens.reserve(tokenValues.size());
  for (const TokenSpan& tokenValue : tokenValues) {
    tokens.push_back(DiffTokenSpan{tokenValue.start, tokenValue.length, tokenValue.syntaxKind});
  }
  return tokens;
}

DiffLayoutMode toLayoutMode(const QString& mode) {
  return mode == "split" ? DiffLayoutMode::Split : DiffLayoutMode::Unified;
}

DiffRowType parseRowType(const QString& value) {
  if (value == "file-header") {
    return DiffRowType::FileHeader;
  }
  return value == "hunk" ? DiffRowType::Hunk : DiffRowType::Line;
}

DiffLineKind parseLineKind(const QString& value) {
  if (value == "add") {
    return DiffLineKind::Addition;
  }
  if (value == "del") {
    return DiffLineKind::Deletion;
  }
  return DiffLineKind::Context;
}

QString kindSymbol(DiffLineKind kind) {
  switch (kind) {
    case DiffLineKind::Addition:
      return "+";
    case DiffLineKind::Deletion:
      return "-";
    default:
      return " ";
  }
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

QObject* DiffSurfaceItem::rowsModel() const {
  return rowsModelObject_;
}

void DiffSurfaceItem::setRowsModel(QObject* model) {
  if (rowsModelObject_ == model) {
    return;
  }
  if (rowsModelObject_ != nullptr) {
    disconnect(rowsModelObject_, nullptr, this, nullptr);
  }

  rowsModelObject_ = model;
  rowsModel_ = qobject_cast<DiffRowListModel*>(model);
  if (rowsModel_ != nullptr) {
    connect(rowsModel_, &QAbstractItemModel::modelReset, this, &DiffSurfaceItem::rebuildRows);
    connect(rowsModel_, &QAbstractItemModel::rowsInserted, this, [this]() { rebuildRows(); });
    connect(rowsModel_, &QAbstractItemModel::rowsRemoved, this, [this]() { rebuildRows(); });
    connect(rowsModel_, &QAbstractItemModel::dataChanged, this, [this]() { rebuildRows(); });
  }

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

QString DiffSurfaceItem::filePath() const {
  return filePath_;
}

void DiffSurfaceItem::setFilePath(const QString& path) {
  if (filePath_ == path) {
    return;
  }
  filePath_ = path;
  rebuildRows();
  emit filePathChanged();
}

QString DiffSurfaceItem::fileStatus() const {
  return fileStatus_;
}

void DiffSurfaceItem::setFileStatus(const QString& status) {
  if (fileStatus_ == status) {
    return;
  }
  fileStatus_ = status;
  rebuildRows();
  emit fileStatusChanged();
}

int DiffSurfaceItem::additions() const {
  return additions_;
}

void DiffSurfaceItem::setAdditions(int value) {
  if (additions_ == value) {
    return;
  }
  additions_ = value;
  rebuildRows();
  emit additionsChanged();
}

int DiffSurfaceItem::deletions() const {
  return deletions_;
}

void DiffSurfaceItem::setDeletions(int value) {
  if (deletions_ == value) {
    return;
  }
  deletions_ = value;
  rebuildRows();
  emit deletionsChanged();
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

qreal DiffSurfaceItem::viewportX() const {
  return viewportX_;
}

void DiffSurfaceItem::setViewportX(qreal value) {
  if (qFuzzyCompare(viewportX_, value)) {
    return;
  }
  viewportX_ = value;
  update();
  emit viewportXChanged();
}

qreal DiffSurfaceItem::viewportY() const {
  return viewportY_;
}

void DiffSurfaceItem::setViewportY(qreal value) {
  if (qFuzzyCompare(viewportY_, value)) {
    return;
  }
  viewportY_ = value;
  const int nextFirst = displayModel_.rowIndexAtY(std::max<qreal>(0.0, viewportY_ - hunkHeight_));
  const int nextLast = displayModel_.rowIndexAtY(viewportY_ + viewportHeight_ + hunkHeight_);
  const int nextSticky = displayModel_.stickyHunkRowIndexAtY(viewportY_);
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
  firstVisibleRow_ = displayModel_.rowIndexAtY(std::max<qreal>(0.0, viewportY_ - hunkHeight_));
  lastVisibleRow_ = displayModel_.rowIndexAtY(viewportY_ + viewportHeight_ + hunkHeight_);
  stickyVisibleRow_ = displayModel_.stickyHunkRowIndexAtY(viewportY_);
  update();
  emit viewportHeightChanged();
}

int DiffSurfaceItem::paintCount() const {
  return paintCount_;
}

int DiffSurfaceItem::displayRowCount() const {
  return displayModel_.rows().size();
}

void DiffSurfaceItem::paint(QPainter* painter) {
  ++paintCount_;

  const QRectF exposedRect = painter->clipBoundingRect();
  painter->fillRect(exposedRect, paletteColor("canvas", QColor("#282c33")));

  const auto& rows = displayModel_.rows();
  if (rows.empty()) {
    return;
  }

  painter->setRenderHint(QPainter::TextAntialiasing, true);
  painter->setRenderHint(QPainter::Antialiasing, false);

  const qreal visibleTop = viewportHeight_ > 0 ? std::max<qreal>(0.0, viewportY_ - hunkHeight_) : exposedRect.top();
  const qreal visibleBottom = viewportHeight_ > 0 ? viewportY_ + viewportHeight_ + hunkHeight_ : exposedRect.bottom();

  int firstRow = displayModel_.rowIndexAtY(visibleTop);
  int lastRow = displayModel_.rowIndexAtY(visibleBottom);
  if (firstRow < 0) {
    firstRow = 0;
  }
  if (lastRow < 0) {
    lastRow = rows.size() - 1;
  }

  for (int rowIndex = firstRow; rowIndex <= lastRow && rowIndex < static_cast<int>(rows.size()); ++rowIndex) {
    const DiffDisplayRow& row = rows.at(rowIndex);
    const QRectF rowRect(-viewportX_, row.top - viewportY_, std::max(width(), contentWidth_), row.height);
    const bool selected = rowSelected(rowIndex);
    const bool hovered = hoveredRow_ == rowIndex;

    if (row.rowType == DiffRowType::FileHeader) {
      drawFileHeaderRow(painter, rowRect, row);
      continue;
    }

    if (row.rowType == DiffRowType::Hunk) {
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
    const int fileHeaderIndex = displayModel_.fileHeaderRowIndex();
    qreal stickyOffset = 0.0;
    if (fileHeaderIndex >= 0 && viewportY_ > 0) {
      painter->save();
      QColor shadow = paletteColor("canvas", QColor("#282828"));
      shadow.setAlpha(225);
      painter->fillRect(QRectF(-viewportX_, 0.0, std::max(width(), contentWidth_), fileHeaderHeight_), shadow);
      drawFileHeaderRow(painter, QRectF(-viewportX_, 0.0, std::max(width(), contentWidth_), fileHeaderHeight_),
                        rows.at(fileHeaderIndex));
      painter->restore();
      stickyOffset = fileHeaderHeight_;
    }

    const int stickyIndex = displayModel_.stickyHunkRowIndexAtY(viewportY_);
    if (stickyIndex >= 0) {
      qreal stickyY = viewportY_ + stickyOffset;
      for (int nextIndex = stickyIndex + 1; nextIndex < static_cast<int>(rows.size()); ++nextIndex) {
        const DiffDisplayRow& nextRow = rows.at(nextIndex);
        if (nextRow.rowType == DiffRowType::Hunk) {
          stickyY = std::min(stickyY, nextRow.top - hunkHeight_);
          break;
        }
      }

      painter->save();
      QColor shadow = paletteColor("canvas", QColor("#282828"));
      shadow.setAlpha(210);
      const qreal stickyViewportY = stickyY - viewportY_;
      painter->fillRect(QRectF(-viewportX_, stickyViewportY, std::max(width(), contentWidth_), hunkHeight_), shadow);
      drawHunkRow(painter, QRectF(-viewportX_, stickyViewportY, std::max(width(), contentWidth_), hunkHeight_), rows.at(stickyIndex));
      painter->restore();
    }
  }
}

void DiffSurfaceItem::rebuildRows() {
  textRope_.clear();
  textCache_.clear();

  const QFontMetricsF metrics(monoFont(monoFontFamily_, 12));
  lineHeight_ = metrics.height();
  rowHeight_ = qCeil(lineHeight_ + 8.0);
  fileHeaderHeight_ = 32.0;
  hunkHeight_ = 28.0;

  std::vector<DiffSourceRow> sourceRows;
  sourceRows.reserve((rowsModel_ != nullptr ? rowsModel_->rows().size() : 0) + (filePath_.isEmpty() ? 0 : 1));

  if (!filePath_.isEmpty()) {
    DiffSourceRow headerRow;
    headerRow.rowType = DiffRowType::FileHeader;
    headerRow.header = filePath_.toStdString();
    headerRow.detail = fileStatus_.toStdString() + " +" + std::to_string(additions_) + " -" +
                       std::to_string(deletions_);
    sourceRows.push_back(std::move(headerRow));
  }

  if (rowsModel_ != nullptr) {
    for (const FlattenedDiffRow& rowValue : rowsModel_->rows()) {
      DiffSourceRow row;
      row.rowType =
          rowValue.rowType == FlattenedDiffRow::RowType::Hunk ? DiffRowType::Hunk : DiffRowType::Line;
      row.header = rowValue.header.toStdString();
      row.kind = rowValue.kind == LineKind::Addition
                     ? DiffLineKind::Addition
                     : rowValue.kind == LineKind::Deletion ? DiffLineKind::Deletion : DiffLineKind::Context;
      row.oldLine = rowValue.oldLine;
      row.newLine = rowValue.newLine;
      const QByteArray textUtf8 = rowValue.text.toUtf8();
      row.textRange = textRope_.append(std::string(textUtf8.constData(), textUtf8.size()));
      row.tokens = parseTokens(rowValue.tokens);
      row.changeSpans = parseTokens(rowValue.changeSpans);
      sourceRows.push_back(std::move(row));
    }
  }

  displayModel_.setSourceRows(std::move(sourceRows));
  rebuildDisplayRows();
}

void DiffSurfaceItem::rebuildDisplayRows() {
  displayModel_.rebuild(toLayoutMode(layoutMode_), rowHeight_, hunkHeight_, fileHeaderHeight_);
  contentHeight_ = displayModel_.contentHeight();
  lineNumberDigits_ = displayModel_.lineNumberDigits();
  maxTextWidth_ = 0;

  const QFontMetricsF widthMetrics(monoFont(monoFontFamily_, 12));
  for (const DiffDisplayRow& row : displayModel_.rows()) {
    maxTextWidth_ = std::max(maxTextWidth_, widthMetrics.horizontalAdvance(textForRange(row.textRange)));
    maxTextWidth_ = std::max(maxTextWidth_, widthMetrics.horizontalAdvance(textForRange(row.leftTextRange)));
    maxTextWidth_ = std::max(maxTextWidth_, widthMetrics.horizontalAdvance(textForRange(row.rightTextRange)));
  }

  emit displayRowCountChanged();
  recalculateMetrics();
}

void DiffSurfaceItem::recalculateMetrics() {
  qreal newContentWidth = 0;
  if (layoutMode_ == "split") {
    const qreal sideGutter = 22.0 + digitWidth() * (lineNumberDigits_ + 1) + 12.0;
    newContentWidth = maxTextWidth_ * 2.0 + sideGutter * 2.0 + 1.0;
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

bool DiffSurfaceItem::rowSelected(int rowIndex) const {
  if (selectionAnchorRow_ < 0 || selectionCursorRow_ < 0) {
    return false;
  }
  const int start = std::min(selectionAnchorRow_, selectionCursorRow_);
  const int end = std::max(selectionAnchorRow_, selectionCursorRow_);
  return rowIndex >= start && rowIndex <= end;
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
  const QFontMetricsF metrics(monoFont(monoFontFamily_, 11));
  return metrics.horizontalAdvance(QLatin1Char('9'));
}

qreal DiffSurfaceItem::unifiedGutterWidth() const {
  return 12.0 + 14.0 + digitWidth() * (lineNumberDigits_ * 2 + 2) + 28.0;
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

int DiffSurfaceItem::currentRowIndex() const {
  if (selectionCursorRow_ >= 0) {
    return selectionCursorRow_;
  }
  if (hoveredRow_ >= 0) {
    return hoveredRow_;
  }
  return displayModel_.rowIndexAtY(viewportY_);
}

void DiffSurfaceItem::drawFileHeaderRow(QPainter* painter, const QRectF& rowRect, const DiffDisplayRow& row) const {
  painter->fillRect(rowRect, paletteColor("canvas", QColor("#282828")));
  painter->fillRect(QRectF(rowRect.left(), rowRect.bottom() - 1.0, rowRect.width(), 1.0),
                    paletteColor("divider", QColor("#363c46")));

  painter->setFont(monoFont(monoFontFamily_, 11));
  painter->setPen(paletteColor("textStrong", QColor("#fbf1c7")));
  painter->drawText(QRectF(rowRect.left() + 12.0, rowRect.top(), rowRect.width() - 160.0, rowRect.height()),
                    Qt::AlignVCenter | Qt::AlignLeft, QString::fromStdString(row.header));

  painter->setPen(paletteColor("textMuted", QColor("#d5c4a1")));
  painter->drawText(QRectF(rowRect.right() - 140.0, rowRect.top(), 130.0, rowRect.height()),
                    Qt::AlignVCenter | Qt::AlignRight, QString::fromStdString(row.detail));
}

void DiffSurfaceItem::drawHunkRow(QPainter* painter, const QRectF& rowRect, const DiffDisplayRow& row) const {
  painter->fillRect(QRectF(rowRect.left(), rowRect.top(), rowRect.width(), 1.0),
                    paletteColor("divider", QColor("#363c46")));
  painter->fillRect(QRectF(rowRect.left(), rowRect.top() + 1.0, rowRect.width(), rowRect.height() - 2.0),
                    paletteColor("panelStrong", QColor("#3b414d")));
  painter->fillRect(QRectF(rowRect.left(), rowRect.bottom() - 1.0, rowRect.width(), 1.0),
                    paletteColor("divider", QColor("#363c46")));

  painter->setFont(monoFont(monoFontFamily_, 11));
  painter->setPen(paletteColor("textMuted", QColor("#a9afbc")));
  painter->drawText(QRectF(rowRect.left() + 10.0, rowRect.top(), rowRect.width() - 20.0, rowRect.height()),
                    Qt::AlignVCenter | Qt::AlignLeft, QString::fromStdString(row.header));
}

void DiffSurfaceItem::drawUnifiedRow(QPainter* painter, const QRectF& rowRect, const DiffDisplayRow& row,
                                     bool selected) const {
  QColor background = paletteColor("lineContext", QColor("#282c33"));
  if (row.kind == DiffLineKind::Addition) {
    background = paletteColor("lineAdd", QColor("#1f2d24"));
  } else if (row.kind == DiffLineKind::Deletion) {
    background = paletteColor("lineDel", QColor("#2d2024"));
  }

  painter->fillRect(rowRect, background);
  if (selected) {
    QColor selection("#45433d");
    if (row.kind == DiffLineKind::Addition) {
      selection = QColor("#3a4a2a");
    } else if (row.kind == DiffLineKind::Deletion) {
      selection = QColor("#4a3030");
    }
    selection.setAlpha(140);
    painter->fillRect(rowRect, selection);
  }

  const qreal gutterWidth = unifiedGutterWidth();
  const QRectF gutterRect(rowRect.left(), rowRect.top(), gutterWidth, rowRect.height());
  painter->fillRect(gutterRect, paletteColor("panelTint", QColor("#353b45")));
  painter->fillRect(QRectF(gutterRect.right(), rowRect.top(), 1.0, rowRect.height()),
                    paletteColor("divider", QColor("#363c46")));

  if (row.kind == DiffLineKind::Addition || row.kind == DiffLineKind::Deletion) {
    const QColor marker = row.kind == DiffLineKind::Addition ? paletteColor("successText", QColor("#a1c181"))
                                                             : paletteColor("dangerText", QColor("#d07277"));
    painter->fillRect(QRectF(rowRect.left(), rowRect.top(), 3.0, rowRect.height()), marker);
  }

  painter->setFont(monoFont(monoFontFamily_, 11));
  painter->setPen(row.kind == DiffLineKind::Addition ? paletteColor("successText", QColor("#a1c181"))
                                                     : row.kind == DiffLineKind::Deletion
                                                           ? paletteColor("dangerText", QColor("#d07277"))
                                                           : paletteColor("textMuted", QColor("#a9afbc")));
  painter->drawText(QRectF(rowRect.left() + 6.0, rowRect.top(), 12.0, rowRect.height()), Qt::AlignVCenter,
                    kindSymbol(row.kind));

  painter->setPen(paletteColor("textMuted", QColor("#d5c4a1")));
  const qreal numberWidth = digitWidth() * lineNumberDigits_;
  painter->drawText(QRectF(rowRect.left() + 22.0, rowRect.top(), numberWidth, rowRect.height()),
                    Qt::AlignRight | Qt::AlignVCenter,
                    row.oldLine > 0 ? QString::number(row.oldLine) : QString());

  painter->fillRect(QRectF(rowRect.left() + 26.0 + numberWidth, rowRect.top() + 4.0, 1.0, rowRect.height() - 8.0),
                    paletteColor("divider", QColor("#504945")));

  painter->drawText(QRectF(rowRect.left() + 32.0 + numberWidth, rowRect.top(), numberWidth, rowRect.height()),
                    Qt::AlignRight | Qt::AlignVCenter,
                    row.newLine > 0 ? QString::number(row.newLine) : QString());

  const QFont textFont = monoFont(monoFontFamily_, 12);
  const QFontMetricsF textMetrics(textFont);
  painter->setFont(textFont);
  const qreal baselineY = rowRect.top() + (rowRect.height() - textMetrics.height()) / 2.0 + textMetrics.ascent();
  const QRectF textClip(rowRect.left() + gutterWidth + 8.0, rowRect.top(),
                        rowRect.width() - gutterWidth - 12.0, rowRect.height());
  const QColor tokenBg = row.kind == DiffLineKind::Addition ? paletteColor("successBorder", QColor("#38482f"))
                                                            : row.kind == DiffLineKind::Deletion
                                                                  ? paletteColor("dangerBorder", QColor("#4c2b2c"))
                                                                  : paletteColor("accentSoft", QColor("#293b5b"));
  drawTextRun(painter, QPointF(textClip.left(), baselineY), textClip, textForRange(row.textRange), row.tokens,
              row.changeSpans, paletteColor("textBase", QColor("#c8ccd4")), tokenBg);
}

void DiffSurfaceItem::drawSplitRow(QPainter* painter, const QRectF& rowRect, const DiffDisplayRow& row,
                                   bool selected) const {
  const QRectF leftRect(rowRect.left(), rowRect.top(), rowRect.width() / 2.0, rowRect.height());
  const QRectF rightRect(leftRect.right(), rowRect.top(), rowRect.width() - leftRect.width(), rowRect.height());
  const qreal sideGutterWidth = 22.0 + digitWidth() * (lineNumberDigits_ + 1) + 12.0;
  const bool leftSpacer = row.leftKind == DiffLineKind::Spacer;
  const bool rightSpacer = row.rightKind == DiffLineKind::Spacer;

  const QColor spacerBg = paletteColor("lineContextAlt", QColor("#232323"));
  const QColor leftBg = leftSpacer ? spacerBg
                                   : row.leftKind == DiffLineKind::Deletion
                                         ? paletteColor("lineDelAccent", QColor("#35262b"))
                                         : paletteColor("lineContext", QColor("#282c33"));
  const QColor rightBg = rightSpacer ? spacerBg
                                     : row.rightKind == DiffLineKind::Addition
                                           ? paletteColor("lineAddAccent", QColor("#22332a"))
                                           : paletteColor("lineContext", QColor("#282c33"));
  painter->fillRect(leftRect, leftBg);
  painter->fillRect(rightRect, rightBg);
  if (selected) {
    QColor leftSelection("#45433d");
    if (row.leftKind == DiffLineKind::Deletion) {
      leftSelection = QColor("#4a3030");
    }
    leftSelection.setAlpha(140);
    painter->fillRect(leftRect, leftSelection);

    QColor rightSelection("#45433d");
    if (row.rightKind == DiffLineKind::Addition) {
      rightSelection = QColor("#3a4a2a");
    }
    rightSelection.setAlpha(140);
    painter->fillRect(rightRect, rightSelection);
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

  if (row.leftKind == DiffLineKind::Deletion) {
    painter->fillRect(QRectF(leftRect.left(), rowRect.top(), 3.0, rowRect.height()),
                      paletteColor("dangerText", QColor("#d07277")));
  }
  if (row.rightKind == DiffLineKind::Addition) {
    painter->fillRect(QRectF(rightRect.left(), rowRect.top(), 3.0, rowRect.height()),
                      paletteColor("successText", QColor("#a1c181")));
  }

  painter->setFont(monoFont(monoFontFamily_, 11));
  painter->setPen(paletteColor("textMuted", QColor("#d5c4a1")));
  painter->drawText(QRectF(leftRect.left() + 6.0, rowRect.top(), 12.0, rowRect.height()), Qt::AlignVCenter,
                    kindSymbol(row.leftKind));
  painter->drawText(QRectF(rightRect.left() + 6.0, rowRect.top(), 12.0, rightRect.height()), Qt::AlignVCenter,
                    kindSymbol(row.rightKind));
  const qreal splitNumberWidth = digitWidth() * lineNumberDigits_;
  painter->drawText(QRectF(leftRect.left() + 20.0, rowRect.top(), splitNumberWidth, rowRect.height()),
                    Qt::AlignRight | Qt::AlignVCenter,
                    row.leftLine > 0 ? QString::number(row.leftLine) : QString());
  painter->drawText(QRectF(rightRect.left() + 20.0, rowRect.top(), splitNumberWidth, rowRect.height()),
                    Qt::AlignRight | Qt::AlignVCenter,
                    row.rightLine > 0 ? QString::number(row.rightLine) : QString());

  const QFont textFont = monoFont(monoFontFamily_, 12);
  const QFontMetricsF textMetrics(textFont);
  painter->setFont(textFont);
  const qreal baselineY = rowRect.top() + (rowRect.height() - textMetrics.height()) / 2.0 + textMetrics.ascent();
  const QRectF leftTextClip(leftRect.left() + sideGutterWidth + 8.0, rowRect.top(),
                            leftRect.width() - sideGutterWidth - 12.0, rowRect.height());
  const QRectF rightTextClip(rightRect.left() + sideGutterWidth + 8.0, rowRect.top(),
                             rightRect.width() - sideGutterWidth - 12.0, rowRect.height());

  if (leftSpacer) {
    QColor guide = paletteColor("divider", QColor("#504945"));
    guide.setAlpha(150);
    painter->fillRect(QRectF(leftTextClip.left(), rowRect.top() + 3.0, 1.0, std::max<qreal>(0.0, rowRect.height() - 6.0)),
                      guide);
  }
  if (rightSpacer) {
    QColor guide = paletteColor("divider", QColor("#504945"));
    guide.setAlpha(150);
    painter->fillRect(QRectF(rightTextClip.left(), rowRect.top() + 3.0, 1.0, std::max<qreal>(0.0, rowRect.height() - 6.0)),
                      guide);
  }

  if (!leftSpacer) {
    drawTextRun(painter, QPointF(leftTextClip.left(), baselineY), leftTextClip,
                textForRange(row.leftTextRange), row.leftTokens, row.leftChangeSpans,
                paletteColor("textBase", QColor("#c8ccd4")),
                paletteColor("dangerBorder", QColor("#4c2b2c")));
  }

  if (!rightSpacer) {
    drawTextRun(painter, QPointF(rightTextClip.left(), baselineY), rightTextClip,
                textForRange(row.rightTextRange), row.rightTokens, row.rightChangeSpans,
                paletteColor("textBase", QColor("#c8ccd4")),
                paletteColor("successBorder", QColor("#38482f")));
  }
}

void DiffSurfaceItem::drawTextRun(QPainter* painter,
                                  const QPointF& baseline,
                                  const QRectF& clipRect,
                                  const QString& text,
                                  const std::vector<DiffTokenSpan>& tokens,
                                  const std::vector<DiffTokenSpan>& changeSpans,
                                  const QColor& textColor,
                                  const QColor& tokenBackground) const {
  painter->save();
  painter->setClipRect(clipRect);

  const QFont textFont = monoFont(monoFontFamily_, 12);
  const QFontMetricsF metrics(textFont);
  painter->setFont(textFont);

  for (const DiffTokenSpan& span : changeSpans) {
    const int start = std::max(0, span.start);
    const int end = std::min(static_cast<int>(text.size()), span.start + span.length);
    if (end <= start) {
      continue;
    }
    const qreal startX = baseline.x() + metrics.horizontalAdvance(text.left(start));
    const qreal spanWidth = metrics.horizontalAdvance(text.mid(start, end - start));
    const QRectF spanRect(startX - 1.0, baseline.y() - metrics.ascent() - 1.0,
                          spanWidth + 2.0, metrics.height() + 2.0);
    painter->fillRect(spanRect, tokenBackground);
  }

  bool hasSyntax = false;
  auto sortedTokens = tokens;
  if (!sortedTokens.empty()) {
    std::sort(sortedTokens.begin(), sortedTokens.end(), [](const DiffTokenSpan& lhs, const DiffTokenSpan& rhs) {
      return lhs.start < rhs.start;
    });
    for (const auto& t : sortedTokens) {
      if (t.syntaxKind != SyntaxTokenKind::None) {
        hasSyntax = true;
        break;
      }
    }
  }

  if (hasSyntax) {
    int cursor = 0;
    for (const DiffTokenSpan& token : sortedTokens) {
      const int tokStart = std::max(0, token.start);
      const int tokEnd = std::min(static_cast<int>(text.size()), token.start + token.length);
      if (tokEnd <= tokStart) {
        continue;
      }
      if (tokStart > cursor) {
        painter->setPen(textColor);
        const qreal x = baseline.x() + metrics.horizontalAdvance(text.left(cursor));
        painter->drawText(QPointF(x, baseline.y()), text.mid(cursor, tokStart - cursor));
      }
      const QColor fg = syntaxForeground(token.syntaxKind);
      painter->setPen(fg.isValid() ? fg : textColor);
      const qreal x = baseline.x() + metrics.horizontalAdvance(text.left(tokStart));
      painter->drawText(QPointF(x, baseline.y()), text.mid(tokStart, tokEnd - tokStart));
      cursor = tokEnd;
    }
    if (cursor < text.size()) {
      painter->setPen(textColor);
      const qreal x = baseline.x() + metrics.horizontalAdvance(text.left(cursor));
      painter->drawText(QPointF(x, baseline.y()), text.mid(cursor));
    }
  } else {
    painter->setPen(textColor);
    painter->drawText(baseline, text);
  }
  painter->restore();
}

QString DiffSurfaceItem::selectedText() const {
  const auto& rows = displayModel_.rows();
  if (selectionAnchorRow_ < 0 || selectionCursorRow_ < 0 || rows.empty()) {
    return {};
  }

  QStringList parts;
  const int start = std::min(selectionAnchorRow_, selectionCursorRow_);
  const int end = std::max(selectionAnchorRow_, selectionCursorRow_);
  for (int rowIndex = start; rowIndex <= end && rowIndex < static_cast<int>(rows.size()); ++rowIndex) {
    const DiffDisplayRow& row = rows.at(rowIndex);
    if (row.rowType == DiffRowType::Hunk) {
      parts.push_back(QString::fromStdString(row.header));
      continue;
    }

    if (layoutMode_ == "split") {
      if (row.leftKind == DiffLineKind::Context && row.rightKind == DiffLineKind::Context) {
        parts.push_back(" " + textForRange(row.leftTextRange));
      } else {
        if (row.leftKind != DiffLineKind::Spacer) {
          parts.push_back("-" + textForRange(row.leftTextRange));
        }
        if (row.rightKind != DiffLineKind::Spacer) {
          parts.push_back("+" + textForRange(row.rightTextRange));
        }
      }
    } else {
      parts.push_back(kindSymbol(row.kind) + textForRange(row.textRange));
    }
  }

  return parts.join('\n');
}

void DiffSurfaceItem::mousePressEvent(QMouseEvent* event) {
  forceActiveFocus(Qt::MouseFocusReason);
  const int rowIndex = displayModel_.rowIndexAtY(event->position().y() + viewportY_);
  selectionAnchorRow_ = rowIndex;
  selectionCursorRow_ = rowIndex;
  update();
  QQuickPaintedItem::mousePressEvent(event);
}

void DiffSurfaceItem::mouseMoveEvent(QMouseEvent* event) {
  if (selectionAnchorRow_ >= 0) {
    selectionCursorRow_ = displayModel_.rowIndexAtY(event->position().y() + viewportY_);
    update();
  }
  QQuickPaintedItem::mouseMoveEvent(event);
}

void DiffSurfaceItem::mouseReleaseEvent(QMouseEvent* event) {
  if (selectionAnchorRow_ >= 0) {
    selectionCursorRow_ = displayModel_.rowIndexAtY(event->position().y() + viewportY_);
    update();
  }
  QQuickPaintedItem::mouseReleaseEvent(event);
}

void DiffSurfaceItem::hoverMoveEvent(QHoverEvent* event) {
  hoveredRow_ = displayModel_.rowIndexAtY(event->position().y() + viewportY_);
  update();
  QQuickPaintedItem::hoverMoveEvent(event);
}

void DiffSurfaceItem::hoverLeaveEvent(QHoverEvent* event) {
  hoveredRow_ = -1;
  update();
  QQuickPaintedItem::hoverLeaveEvent(event);
}

void DiffSurfaceItem::keyPressEvent(QKeyEvent* event) {
  const auto& rows = displayModel_.rows();

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
    if (!rows.empty()) {
      selectionAnchorRow_ = 0;
      selectionCursorRow_ = static_cast<int>(rows.size()) - 1;
      update();
    }
    event->accept();
    return;
  }

  if (rows.empty()) {
    QQuickPaintedItem::keyPressEvent(event);
    return;
  }

  const int rowIndex = std::clamp(currentRowIndex(), 0, static_cast<int>(rows.size()) - 1);
  const DiffDisplayRow& row = rows.at(rowIndex);

  if (event->key() == Qt::Key_PageDown) {
    emit scrollToYRequested(viewportY_ + viewportHeight_ * 0.9);
    event->accept();
    return;
  }

  if (event->key() == Qt::Key_PageUp) {
    emit scrollToYRequested(std::max<qreal>(0.0, viewportY_ - viewportHeight_ * 0.9));
    event->accept();
    return;
  }

  if (event->key() == Qt::Key_Home) {
    emit scrollToYRequested(0.0);
    event->accept();
    return;
  }

  if (event->key() == Qt::Key_End) {
    emit scrollToYRequested(std::max<qreal>(0.0, contentHeight_ - viewportHeight_));
    event->accept();
    return;
  }

  if (event->modifiers() == Qt::AltModifier && event->key() == Qt::Key_Down) {
    const int nextHunk = displayModel_.nextHunkRowIndex(rowIndex);
    if (nextHunk >= 0) {
      selectionAnchorRow_ = nextHunk;
      selectionCursorRow_ = nextHunk;
      emit scrollToYRequested(rows.at(nextHunk).top);
      update();
    }
    event->accept();
    return;
  }

  if (event->modifiers() == Qt::AltModifier && event->key() == Qt::Key_Up) {
    const int previousHunk = displayModel_.previousHunkRowIndex(rowIndex);
    if (previousHunk >= 0) {
      selectionAnchorRow_ = previousHunk;
      selectionCursorRow_ = previousHunk;
      emit scrollToYRequested(rows.at(previousHunk).top);
      update();
    }
    event->accept();
    return;
  }

  QQuickPaintedItem::keyPressEvent(event);
}

}  // namespace diffy
