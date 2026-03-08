#include "ui/DiffSurfaceItem.h"

#include <QClipboard>
#include <QColor>
#include <QFont>
#include <QFontMetricsF>
#include <QGuiApplication>
#include <QImage>
#include <QPainter>
#include <QQuickWindow>
#include <QRunnable>
#include <QSGClipNode>
#include <QSGGeometry>
#include <QSGNode>
#include <QSGSimpleRectNode>
#include <QSGSimpleTextureNode>
#include <QStyleHints>
#include <QSet>
#include <QTimer>
#include <QVector>
#include <QtMath>

#include <algorithm>
#include <limits>

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

qreal wheelStepPixels(int pixelDelta, int angleDelta, qreal lineStep) {
  if (pixelDelta != 0) {
    return static_cast<qreal>(pixelDelta);
  }
  if (angleDelta == 0) {
    return 0.0;
  }

  const QStyleHints* styleHints = QGuiApplication::styleHints();
  const int wheelLines = styleHints != nullptr ? std::max(1, styleHints->wheelScrollLines()) : 3;
  return angleDelta / 120.0 * lineStep * wheelLines;
}

std::vector<qreal> prefixAdvances(const QString& text, const QFontMetricsF& metrics) {
  std::vector<qreal> positions(text.size() + 1, 0.0);
  for (int i = 0; i < text.size(); ++i) {
    positions[i + 1] = positions[i] + metrics.horizontalAdvance(text.at(i));
  }
  return positions;
}

constexpr int kRowTileWidth = 1024;
constexpr int kColumnPrefetchMargin = 1;
constexpr int kMaxResidentTiles = 512;
constexpr int kMaxRasterTiles = 1024;

enum class TileLayer {
  UnifiedRow = 1,
  SplitFullRow = 2,
  SplitLeftFixedRow = 3,
  SplitRightFixedRow = 4,
  SplitLeftTextRow = 5,
  SplitRightTextRow = 6,
  StickyRow = 7,
};

quint64 tileKey(quint64 generation, TileLayer layer, int rowIndex, int columnIndex) {
  quint64 hash = generation * 0x9e3779b97f4a7c15ULL;
  hash ^= static_cast<quint64>(layer) + 0x9e3779b97f4a7c15ULL + (hash << 6) + (hash >> 2);
  hash ^= static_cast<quint64>(rowIndex + 1) + 0x9e3779b97f4a7c15ULL + (hash << 6) + (hash >> 2);
  hash ^= static_cast<quint64>(columnIndex + 1) + 0x9e3779b97f4a7c15ULL + (hash << 6) + (hash >> 2);
  return hash;
}

quint64 overlayKey(int layer, int rowIndex, int slot) {
  quint64 hash = 0xd1f7000000000000ULL;
  hash ^= static_cast<quint64>(layer + 1) * 0x9e3779b97f4a7c15ULL;
  hash ^= static_cast<quint64>(rowIndex + 2) * 0xbf58476d1ce4e5b9ULL;
  hash ^= static_cast<quint64>(slot + 1) * 0x94d049bb133111ebULL;
  return hash;
}

struct TileSpec {
  quint64 key = 0;
  TileLayer layer = TileLayer::UnifiedRow;
  int rowIndex = -1;
  int columnIndex = 0;
  qreal logicalX = 0.0;
  QRectF targetRect;
};

class TextureTileNode final : public QSGSimpleTextureNode {
 public:
  quint64 key = 0;
};

class RectTileNode final : public QSGSimpleRectNode {
 public:
  quint64 key = 0;
};

class LayerGroupNode final : public QSGNode {
 public:
  int layer = 0;
};

class TextClipNode final : public QSGClipNode {
 public:
  int layer = 0;
};

void updateClipNodeRect(QSGClipNode* clipNode, const QRectF& rect) {
  if (clipNode == nullptr) {
    return;
  }
  auto* geometry = clipNode->geometry();
  if (geometry == nullptr) {
    geometry = new QSGGeometry(QSGGeometry::defaultAttributes_Point2D(), 4);
    geometry->setDrawingMode(QSGGeometry::DrawTriangleStrip);
    clipNode->setGeometry(geometry);
    clipNode->setFlag(QSGNode::OwnsGeometry);
  }
  QSGGeometry::updateRectGeometry(geometry, rect);
  clipNode->setIsRectangular(true);
  clipNode->setClipRect(rect);
}

QSGClipNode* createClipNode(const QRectF& rect) {
  auto* clipNode = new QSGClipNode;
  updateClipNodeRect(clipNode, rect);
  return clipNode;
}

class TextureCleanupJob final : public QRunnable {
 public:
  explicit TextureCleanupJob(QVector<QSGTexture*> textures) : textures_(std::move(textures)) {}

  void run() override {
    qDeleteAll(textures_);
  }

 private:
  QVector<QSGTexture*> textures_;
};

QColor unifiedSelectionColor(const DiffDisplayRow& row) {
  QColor selection("#45433d");
  if (row.kind == DiffLineKind::Addition) {
    selection = QColor("#3a4a2a");
  } else if (row.kind == DiffLineKind::Deletion) {
    selection = QColor("#4a3030");
  }
  selection.setAlpha(140);
  return selection;
}

QColor splitSelectionColor(const DiffDisplayRow& row, bool isLeftPane) {
  QColor selection("#45433d");
  if (isLeftPane && row.leftKind == DiffLineKind::Deletion) {
    selection = QColor("#4a3030");
  } else if (!isLeftPane && row.rightKind == DiffLineKind::Addition) {
    selection = QColor("#3a4a2a");
  }
  selection.setAlpha(140);
  return selection;
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

DiffSurfaceItem::DiffSurfaceItem(QQuickItem* parent) : QQuickItem(parent) {
  setFlag(ItemHasContents, true);
  setAcceptedMouseButtons(Qt::LeftButton);
  setAcceptHoverEvents(true);
  setFocus(true);
  connect(this, &QQuickItem::widthChanged, this, [this]() { recalculateMetrics(); });
  connect(this, &QQuickItem::heightChanged, this, [this]() { update(); });
}

QObject* DiffSurfaceItem::rowsModel() const {
  return rowsModelObject_;
}

void DiffSurfaceItem::invalidateTiles() {
  ++tileGeneration_;
  update();
  updateTileStats();
}

void DiffSurfaceItem::updateTileStats() {
  emit tileStatsChanged();
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
  leftViewportX_ = 0;
  rightViewportX_ = 0;
  recalculateMetrics();
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
  invalidateTiles();
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
  rebuildRows();
  emit monoFontFamilyChanged();
}

bool DiffSurfaceItem::wrapEnabled() const {
  return wrapEnabled_;
}

void DiffSurfaceItem::setWrapEnabled(bool value) {
  if (wrapEnabled_ == value) return;
  wrapEnabled_ = value;
  if (wrapEnabled_) {
    leftViewportX_ = 0;
    rightViewportX_ = 0;
  }
  recalculateMetrics();
  emit wrapEnabledChanged();
}

int DiffSurfaceItem::wrapColumn() const {
  return wrapColumn_;
}

void DiffSurfaceItem::setWrapColumn(int value) {
  if (wrapColumn_ == value) return;
  wrapColumn_ = value;
  if (wrapEnabled_) recalculateMetrics();
  emit wrapColumnChanged();
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

qreal DiffSurfaceItem::leftViewportX() const {
  return leftViewportX_;
}

void DiffSurfaceItem::setLeftViewportX(qreal value) {
  if (wrapEnabled_) {
    value = 0.0;
  }
  value = std::max(0.0, value);
  if (qFuzzyCompare(leftViewportX_, value)) return;
  leftViewportX_ = value;
  update();
  emit leftViewportXChanged();
}

qreal DiffSurfaceItem::rightViewportX() const {
  return rightViewportX_;
}

void DiffSurfaceItem::setRightViewportX(qreal value) {
  if (wrapEnabled_) {
    value = 0.0;
  }
  value = std::max(0.0, value);
  if (qFuzzyCompare(rightViewportX_, value)) return;
  rightViewportX_ = value;
  update();
  emit rightViewportXChanged();
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
  firstVisibleRow_ = nextFirst;
  lastVisibleRow_ = nextLast;
  stickyVisibleRow_ = nextSticky;
  update();
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

int DiffSurfaceItem::tileCacheHits() const {
  return tileCacheHits_;
}

int DiffSurfaceItem::tileCacheMisses() const {
  return tileCacheMisses_;
}

int DiffSurfaceItem::textureUploadCount() const {
  return textureUploadCount_;
}

int DiffSurfaceItem::residentTileCount() const {
  return residentTextureCache_.size();
}

int DiffSurfaceItem::pendingTileJobCount() const {
  return pendingTileJobCount_;
}

QSGNode* DiffSurfaceItem::updatePaintNode(QSGNode* oldNode, UpdatePaintNodeData* data) {
  Q_UNUSED(data);

  auto* root = oldNode != nullptr ? oldNode : new QSGNode;
  auto clearChildren = [](QSGNode* parent) {
    while (QSGNode* child = parent->firstChild()) {
      parent->removeChildNode(child);
      delete child;
    }
  };

  auto* quickWindow = window();
  const auto& rows = displayModel_.rows();
  if (quickWindow == nullptr || width() <= 0 || height() <= 0 || rows.empty()) {
    clearChildren(root);
    return root;
  }

  const int uploadsBefore = textureUploadCount_;
  ++paintCount_;
  emit paintCountChanged();

  const qreal visibleWidth = width();
  const qreal visibleHeight = height();
  const qreal unifiedRowWidth = std::max(visibleWidth, contentWidth_);
  const qreal leftPaneWidth = visibleWidth / 2.0;
  const qreal rightPaneWidth = visibleWidth - leftPaneWidth;
  const qreal sideGutterWidth = 22.0 + digitWidth() * (lineNumberDigits_ + 1) + 12.0;
  const qreal splitTextInset = sideGutterWidth + 8.0;
  const qreal leftTextViewportWidth = std::max<qreal>(0.0, leftPaneWidth - splitTextInset - 12.0);
  const qreal rightTextViewportWidth = std::max<qreal>(0.0, rightPaneWidth - splitTextInset - 12.0);
  const qreal splitTextLogicalWidth = std::max({maxTextWidth_ + 12.0, leftTextViewportWidth, rightTextViewportWidth});
  const qreal devicePixelRatio = quickWindow->effectiveDevicePixelRatio();

  struct RenderCommand {
    enum class Kind {
      Rect,
      Texture,
    };

    Kind kind = Kind::Rect;
    quint64 key = 0;
    QRectF rect;
    QColor color;
    TileSpec spec;
  };

  std::vector<RenderCommand> baseCommands;
  std::vector<RenderCommand> leftTextCommands;
  std::vector<RenderCommand> rightTextCommands;
  std::vector<RenderCommand> overlayCommands;
  std::vector<RenderCommand> stickyCommands;

  auto queueRectNode = [&](std::vector<RenderCommand>& commands, quint64 key, const QRectF& rect, const QColor& color) {
    if (!rect.isValid() || rect.width() <= 0.0 || rect.height() <= 0.0 || !color.isValid() || color.alpha() == 0) {
      return;
    }
    RenderCommand command;
    command.kind = RenderCommand::Kind::Rect;
    command.key = key;
    command.rect = rect;
    command.color = color;
    commands.push_back(std::move(command));
  };

  auto renderImage = [&](const TileSpec& spec) -> QImage {
    const DiffDisplayRow& row = rows.at(spec.rowIndex);
    const QSize pixelSize(qMax(1, qCeil(spec.targetRect.width() * devicePixelRatio)),
                          qMax(1, qCeil(row.height * devicePixelRatio)));
    QImage image(pixelSize, QImage::Format_ARGB32_Premultiplied);
    image.setDevicePixelRatio(devicePixelRatio);
    image.fill(paletteColor("canvas", QColor("#282828")).rgb());

    QPainter painter(&image);
    painter.setRenderHint(QPainter::TextAntialiasing, true);
    painter.setRenderHint(QPainter::Antialiasing, false);
    painter.translate(-spec.logicalX, -row.top);

    switch (spec.layer) {
      case TileLayer::UnifiedRow: {
        const QRectF rowRect(0.0, row.top, unifiedRowWidth, row.height);
        if (row.rowType == DiffRowType::FileHeader) {
          drawFileHeaderRow(&painter, rowRect, row);
        } else if (row.rowType == DiffRowType::Hunk) {
          drawHunkRow(&painter, rowRect, row);
        } else {
          drawUnifiedRow(&painter, rowRect, row, false);
        }
        break;
      }
      case TileLayer::SplitFullRow: {
        const QRectF rowRect(0.0, row.top, visibleWidth, row.height);
        if (row.rowType == DiffRowType::FileHeader) {
          drawFileHeaderRow(&painter, rowRect, row);
        } else if (row.rowType == DiffRowType::Hunk) {
          drawHunkRow(&painter, rowRect, row);
        } else {
          drawSplitRow(&painter, rowRect, row, false, leftViewportX_, rightViewportX_);
        }
        break;
      }
      case TileLayer::SplitLeftFixedRow:
      case TileLayer::SplitRightFixedRow: {
        const bool isLeftPane = spec.layer == TileLayer::SplitLeftFixedRow;
        const qreal paneWidth = isLeftPane ? leftPaneWidth : rightPaneWidth;
        drawSplitPaneFixedRow(&painter, QRectF(0.0, row.top, paneWidth, row.height), row, isLeftPane, false);
        break;
      }
      case TileLayer::SplitLeftTextRow:
      case TileLayer::SplitRightTextRow: {
        const bool isLeftPane = spec.layer == TileLayer::SplitLeftTextRow;
        drawSplitPaneTextRow(&painter, QRectF(0.0, row.top, splitTextLogicalWidth, row.height), row, isLeftPane);
        break;
      }
      case TileLayer::StickyRow: {
        const QRectF rowRect(0.0, row.top, spec.targetRect.width(), row.height);
        if (row.rowType == DiffRowType::FileHeader) {
          drawFileHeaderRow(&painter, rowRect, row);
        } else if (row.rowType == DiffRowType::Hunk) {
          drawHunkRow(&painter, rowRect, row);
        }
        break;
      }
    }
    painter.end();

    return image;
  };

  QSet<quint64> pinnedKeys;

  auto residentTextureForSpec = [&](const TileSpec& spec) -> QSGTexture* {
    ++tileUseTick_;
    pinnedKeys.insert(spec.key);

    if (auto it = residentTextureCache_.find(spec.key); it != residentTextureCache_.end() && it.value() != nullptr) {
      residentTextureLastUsed_[spec.key] = tileUseTick_;
      ++tileCacheHits_;
      return it.value();
    }

    QImage image;
    if (auto imageIt = tileImageCache_.find(spec.key); imageIt != tileImageCache_.end()) {
      image = imageIt.value();
      ++tileCacheHits_;
    } else {
      image = renderImage(spec);
      tileImageCache_.insert(spec.key, image);
      ++tileCacheMisses_;
    }
    tileImageLastUsed_[spec.key] = tileUseTick_;

    QSGTexture* texture = quickWindow->createTextureFromImage(image);
    residentTextureCache_.insert(spec.key, texture);
    residentTextureLastUsed_[spec.key] = tileUseTick_;
    ++textureUploadCount_;
    return texture;
  };

  auto queueTextureNode = [&](std::vector<RenderCommand>& commands, const TileSpec& spec) {
    if (spec.targetRect.width() <= 0.0 || spec.targetRect.height() <= 0.0) {
      return;
    }
    RenderCommand command;
    command.kind = RenderCommand::Kind::Texture;
    command.key = spec.key;
    command.rect = spec.targetRect;
    command.spec = spec;
    commands.push_back(std::move(command));
  };

  auto queueWholeRowTexture = [&](std::vector<RenderCommand>& commands,
                                  TileLayer layer,
                                  int rowIndex,
                                  qreal logicalX,
                                  const QRectF& targetRect) {
    TileSpec spec;
    spec.layer = layer;
    spec.rowIndex = rowIndex;
    spec.columnIndex = 0;
    spec.logicalX = logicalX;
    spec.key = tileKey(tileGeneration_, layer, rowIndex, 0);
    spec.targetRect = targetRect;
    queueTextureNode(commands, spec);
  };

  auto queueTileColumns = [&](std::vector<RenderCommand>& commands,
                              TileLayer layer,
                              int rowIndex,
                              qreal logicalOffset,
                              qreal viewportSpan,
                              qreal logicalWidth,
                              qreal baseX,
                              qreal y) {
    if (viewportSpan <= 0.0 || logicalWidth <= 0.0) {
      return;
    }

    const int maxColumn = std::max(0, static_cast<int>(std::ceil(logicalWidth / kRowTileWidth)) - 1);
    const int firstColumn =
        std::clamp(static_cast<int>(std::floor(logicalOffset / kRowTileWidth)) - kColumnPrefetchMargin, 0, maxColumn);
    const int lastColumn = std::clamp(
        static_cast<int>(std::floor((logicalOffset + viewportSpan - 1.0) / kRowTileWidth)) + kColumnPrefetchMargin,
        firstColumn, maxColumn);

    for (int columnIndex = firstColumn; columnIndex <= lastColumn; ++columnIndex) {
      const qreal logicalX = columnIndex * kRowTileWidth;
      const qreal tileWidth = std::min<qreal>(kRowTileWidth, logicalWidth - logicalX);
      if (tileWidth <= 0.0) {
        continue;
      }

      TileSpec spec;
      spec.layer = layer;
      spec.rowIndex = rowIndex;
      spec.columnIndex = columnIndex;
      spec.logicalX = logicalX;
      spec.key = tileKey(tileGeneration_, layer, rowIndex, columnIndex);
      spec.targetRect = QRectF(baseX + logicalX - logicalOffset, y, tileWidth, rows.at(rowIndex).height);
      queueTextureNode(commands, spec);
    }
  };

  const int firstRow = firstVisibleRow_ >= 0
                           ? firstVisibleRow_
                           : std::max(0, displayModel_.rowIndexAtY(std::max<qreal>(0.0, viewportY_ - hunkHeight_)));
  const int lastRow = lastVisibleRow_ >= 0
                          ? lastVisibleRow_
                          : std::max(firstRow, displayModel_.rowIndexAtY(viewportY_ + visibleHeight + hunkHeight_));
  baseCommands.reserve(static_cast<size_t>(std::max(16, lastRow - firstRow + 8)));
  leftTextCommands.reserve(static_cast<size_t>(std::max(8, lastRow - firstRow + 4)));
  rightTextCommands.reserve(static_cast<size_t>(std::max(8, lastRow - firstRow + 4)));
  overlayCommands.reserve(static_cast<size_t>(std::max(8, lastRow - firstRow + 4)));
  stickyCommands.reserve(4);
  QSGClipNode* rootClip = dynamic_cast<QSGClipNode*>(root->firstChild());
  if (rootClip == nullptr) {
    clearChildren(root);
    rootClip = createClipNode(QRectF(0.0, 0.0, visibleWidth, visibleHeight));
    root->appendChildNode(rootClip);
  } else {
    updateClipNodeRect(rootClip, QRectF(0.0, 0.0, visibleWidth, visibleHeight));
    while (QSGNode* extra = rootClip->nextSibling()) {
      root->removeChildNode(extra);
      delete extra;
    }
  }

  LayerGroupNode* baseGroup = dynamic_cast<LayerGroupNode*>(rootClip->firstChild());
  TextClipNode* leftTextClip = baseGroup != nullptr ? dynamic_cast<TextClipNode*>(baseGroup->nextSibling()) : nullptr;
  TextClipNode* rightTextClip =
      leftTextClip != nullptr ? dynamic_cast<TextClipNode*>(leftTextClip->nextSibling()) : nullptr;
  LayerGroupNode* overlayGroup =
      rightTextClip != nullptr ? dynamic_cast<LayerGroupNode*>(rightTextClip->nextSibling()) : nullptr;
  LayerGroupNode* stickyGroup =
      overlayGroup != nullptr ? dynamic_cast<LayerGroupNode*>(overlayGroup->nextSibling()) : nullptr;

  const bool validLayerTree = baseGroup != nullptr && baseGroup->layer == 1 && leftTextClip != nullptr &&
                              leftTextClip->layer == 2 && rightTextClip != nullptr && rightTextClip->layer == 3 &&
                              overlayGroup != nullptr && overlayGroup->layer == 4 && stickyGroup != nullptr &&
                              stickyGroup->layer == 5 && stickyGroup->nextSibling() == nullptr;

  if (!validLayerTree) {
    clearChildren(rootClip);

    baseGroup = new LayerGroupNode;
    baseGroup->layer = 1;
    rootClip->appendChildNode(baseGroup);

    leftTextClip = new TextClipNode;
    leftTextClip->layer = 2;
    rootClip->appendChildNode(leftTextClip);

    rightTextClip = new TextClipNode;
    rightTextClip->layer = 3;
    rootClip->appendChildNode(rightTextClip);

    overlayGroup = new LayerGroupNode;
    overlayGroup->layer = 4;
    rootClip->appendChildNode(overlayGroup);

    stickyGroup = new LayerGroupNode;
    stickyGroup->layer = 5;
    rootClip->appendChildNode(stickyGroup);
  }

  updateClipNodeRect(leftTextClip, QRectF(splitTextInset, 0.0, leftTextViewportWidth, visibleHeight));
  updateClipNodeRect(rightTextClip,
                     QRectF(leftPaneWidth + splitTextInset, 0.0, rightTextViewportWidth, visibleHeight));

  queueRectNode(baseCommands, overlayKey(0, -1, 0), QRectF(0.0, 0.0, visibleWidth, visibleHeight),
                paletteColor("canvas", QColor("#282c33")));

  if (layoutMode_ == "split") {
    for (int rowIndex = firstRow; rowIndex <= lastRow && rowIndex < static_cast<int>(rows.size()); ++rowIndex) {
      const DiffDisplayRow& row = rows.at(rowIndex);
      const qreal y = row.top - viewportY_;
      if (y + row.height < 0.0 || y > visibleHeight) {
        continue;
      }

      if (row.rowType == DiffRowType::Line) {
        queueWholeRowTexture(baseCommands, TileLayer::SplitLeftFixedRow, rowIndex, 0.0,
                             QRectF(0.0, y, leftPaneWidth, row.height));
        queueWholeRowTexture(baseCommands, TileLayer::SplitRightFixedRow, rowIndex, 0.0,
                             QRectF(leftPaneWidth, y, rightPaneWidth, row.height));
        queueTileColumns(leftTextCommands, TileLayer::SplitLeftTextRow, rowIndex, leftViewportX_, leftTextViewportWidth,
                         splitTextLogicalWidth, splitTextInset, y);
        queueTileColumns(rightTextCommands, TileLayer::SplitRightTextRow, rowIndex, rightViewportX_,
                         rightTextViewportWidth, splitTextLogicalWidth, leftPaneWidth + splitTextInset, y);
      } else {
        queueWholeRowTexture(baseCommands, TileLayer::SplitFullRow, rowIndex, 0.0,
                             QRectF(0.0, y, visibleWidth, row.height));
      }
      if (row.rowType == DiffRowType::Line && (rowSelected(rowIndex) || hoveredRow_ == rowIndex)) {
        const qreal leftPaneWidth = visibleWidth / 2.0;
        const qreal rightPaneWidth = visibleWidth - leftPaneWidth;
        queueRectNode(overlayCommands, overlayKey(1, rowIndex, 0), QRectF(0.0, y, leftPaneWidth, row.height),
                      splitSelectionColor(row, true));
        queueRectNode(overlayCommands, overlayKey(1, rowIndex, 1), QRectF(leftPaneWidth, y, rightPaneWidth, row.height),
                      splitSelectionColor(row, false));
      } else if (row.rowType == DiffRowType::Hunk) {
        QColor overlay;
        if (rowSelected(rowIndex)) {
          overlay = paletteColor("selectionBg", QColor("#3c3836"));
          overlay.setAlpha(110);
        } else if (hoveredRow_ == rowIndex) {
          overlay = paletteColor("panelTint", QColor("#504945"));
          overlay.setAlpha(90);
        }
        queueRectNode(overlayCommands, overlayKey(2, rowIndex, 0), QRectF(0.0, y, visibleWidth, row.height), overlay);
      }
    }
  } else {
    for (int rowIndex = firstRow; rowIndex <= lastRow && rowIndex < static_cast<int>(rows.size()); ++rowIndex) {
      const DiffDisplayRow& row = rows.at(rowIndex);
      const qreal y = row.top - viewportY_;
      if (y + row.height < 0.0 || y > visibleHeight) {
        continue;
      }

      queueTileColumns(baseCommands, TileLayer::UnifiedRow, rowIndex, viewportX_, visibleWidth, unifiedRowWidth, 0.0,
                       y);
      if (row.rowType == DiffRowType::Line && (rowSelected(rowIndex) || hoveredRow_ == rowIndex)) {
        queueRectNode(overlayCommands, overlayKey(3, rowIndex, 0), QRectF(-viewportX_, y, unifiedRowWidth, row.height),
                      unifiedSelectionColor(row));
      } else if (row.rowType == DiffRowType::Hunk) {
        QColor overlay;
        if (rowSelected(rowIndex)) {
          overlay = paletteColor("selectionBg", QColor("#3c3836"));
          overlay.setAlpha(110);
        } else if (hoveredRow_ == rowIndex) {
          overlay = paletteColor("panelTint", QColor("#504945"));
          overlay.setAlpha(90);
        }
        queueRectNode(overlayCommands, overlayKey(4, rowIndex, 0), QRectF(-viewportX_, y, unifiedRowWidth, row.height),
                      overlay);
      }
    }
  }

  const int fileHeaderIndex = displayModel_.fileHeaderRowIndex();
  qreal stickyOffset = 0.0;
  if (fileHeaderIndex >= 0 && viewportY_ > 0) {
    queueRectNode(stickyCommands, overlayKey(5, fileHeaderIndex, 0), QRectF(0.0, 0.0, visibleWidth, fileHeaderHeight_),
                  paletteColor("canvas", QColor("#282828")));
    const TileSpec stickyHeaderSpec{tileKey(tileGeneration_,
                                            TileLayer::StickyRow,
                                            fileHeaderIndex, 0),
                                    TileLayer::StickyRow,
                                    fileHeaderIndex,
                                    0,
                                    0.0,
                                    QRectF(0.0, 0.0, visibleWidth, rows.at(fileHeaderIndex).height)};
    queueTextureNode(stickyCommands, stickyHeaderSpec);
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

    const qreal stickyViewportY = stickyY - viewportY_;
    queueRectNode(stickyCommands, overlayKey(6, stickyIndex, 0), QRectF(0.0, stickyViewportY, visibleWidth, hunkHeight_),
                  paletteColor("canvas", QColor("#282828")));
    const TileSpec stickyHunkSpec{tileKey(tileGeneration_,
                                          TileLayer::StickyRow,
                                          stickyIndex, 0),
                                  TileLayer::StickyRow,
                                  stickyIndex,
                                  0,
                                  0.0,
                                  QRectF(0.0, stickyViewportY, visibleWidth, rows.at(stickyIndex).height)};
    queueTextureNode(stickyCommands, stickyHunkSpec);
  }

  auto updateRectNode = [](RectTileNode* node, quint64 key, const QRectF& rect, const QColor& color) {
    node->key = key;
    node->setRect(rect);
    node->setColor(color);
  };

  auto updateTextureNode = [&](TextureTileNode* node, const TileSpec& spec) {
    node->key = spec.key;
    node->setFiltering(QSGTexture::Nearest);
    node->setOwnsTexture(false);
    node->setTexture(residentTextureForSpec(spec));
    node->setRect(spec.targetRect);
  };

  auto reconcileCommands = [&](QSGNode* parent, const std::vector<RenderCommand>& commands) {
    QSGNode* cursor = parent->firstChild();
    for (const RenderCommand& command : commands) {
      if (command.kind == RenderCommand::Kind::Texture) {
        auto* node = cursor != nullptr ? dynamic_cast<TextureTileNode*>(cursor) : nullptr;
        if (node != nullptr && node->key == command.key) {
          updateTextureNode(node, command.spec);
          cursor = cursor->nextSibling();
          continue;
        }

        auto* newNode = new TextureTileNode;
        updateTextureNode(newNode, command.spec);
        if (cursor != nullptr) {
          parent->insertChildNodeBefore(newNode, cursor);
        } else {
          parent->appendChildNode(newNode);
        }
        continue;
      }

      auto* node = cursor != nullptr ? dynamic_cast<RectTileNode*>(cursor) : nullptr;
      if (node != nullptr && node->key == command.key) {
        updateRectNode(node, command.key, command.rect, command.color);
        cursor = cursor->nextSibling();
        continue;
      }

      auto* newNode = new RectTileNode;
      updateRectNode(newNode, command.key, command.rect, command.color);
      if (cursor != nullptr) {
        parent->insertChildNodeBefore(newNode, cursor);
      } else {
        parent->appendChildNode(newNode);
      }
    }

    while (cursor != nullptr) {
      QSGNode* next = cursor->nextSibling();
      parent->removeChildNode(cursor);
      delete cursor;
      cursor = next;
    }
  };

  reconcileCommands(baseGroup, baseCommands);
  reconcileCommands(leftTextClip, leftTextCommands);
  reconcileCommands(rightTextClip, rightTextCommands);
  reconcileCommands(overlayGroup, overlayCommands);
  reconcileCommands(stickyGroup, stickyCommands);

  auto evictCache = [&](int limit, auto& cache, auto& lastUsed, auto deleter) {
    while (cache.size() > limit) {
      quint64 victimKey = 0;
      quint64 victimTick = std::numeric_limits<quint64>::max();
      bool foundVictim = false;
      for (auto it = cache.begin(); it != cache.end(); ++it) {
        if (pinnedKeys.contains(it.key())) {
          continue;
        }
        const quint64 keyTick = lastUsed.value(it.key(), 0);
        if (!foundVictim || keyTick < victimTick) {
          foundVictim = true;
          victimKey = it.key();
          victimTick = keyTick;
        }
      }
      if (!foundVictim) {
        break;
      }
      deleter(victimKey);
    }
  };

  evictCache(kMaxRasterTiles, tileImageCache_, tileImageLastUsed_, [&](quint64 key) {
    tileImageCache_.remove(key);
    tileImageLastUsed_.remove(key);
  });
  evictCache(kMaxResidentTiles, residentTextureCache_, residentTextureLastUsed_, [&](quint64 key) {
    delete residentTextureCache_.take(key);
    residentTextureLastUsed_.remove(key);
  });

  if (textureUploadCount_ > uploadsBefore && !followupUpdateQueued_) {
    followupUpdateQueued_ = true;
    QTimer::singleShot(16, this, [this]() {
      followupUpdateQueued_ = false;
      update();
    });
  }

  updateTileStats();

  return root;
}

void DiffSurfaceItem::releaseResources() {
  QVector<QSGTexture*> textures;
  textures.reserve(residentTextureCache_.size());
  for (auto it = residentTextureCache_.begin(); it != residentTextureCache_.end(); ++it) {
    if (it.value() != nullptr) {
      textures.push_back(it.value());
    }
  }

  residentTextureCache_.clear();
  residentTextureLastUsed_.clear();
  tileImageCache_.clear();
  tileImageLastUsed_.clear();
  updateTileStats();

  if (!textures.isEmpty()) {
    if (QQuickWindow* quickWindow = window()) {
      quickWindow->scheduleRenderJob(new TextureCleanupJob(std::move(textures)),
                                     QQuickWindow::BeforeSynchronizingStage);
    } else {
      qDeleteAll(textures);
    }
  }

  QQuickItem::releaseResources();
}

void DiffSurfaceItem::rebuildRows() {
  textRope_.clear();
  textCache_.clear();
  leftViewportX_ = 0;
  rightViewportX_ = 0;

  const QFontMetricsF metrics(monoFont(monoFontFamily_, 12));
  lineHeight_ = metrics.height();
  rowHeight_ = qCeil(lineHeight_ + 8.0);
  fileHeaderHeight_ = 32.0;
  hunkHeight_ = 28.0;
  maxTextWidth_ = 0;

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
      row.textWidth = metrics.horizontalAdvance(rowValue.text);
      row.textRange = textRope_.append(std::string(textUtf8.constData(), textUtf8.size()));
      row.tokens = parseTokens(rowValue.tokens);
      row.changeSpans = parseTokens(rowValue.changeSpans);
      maxTextWidth_ = std::max(maxTextWidth_, row.textWidth);
      sourceRows.push_back(std::move(row));
    }
  }

  displayModel_.setSourceRows(std::move(sourceRows));
  recalculateMetrics();
}

void DiffSurfaceItem::rebuildDisplayRows() {
  DiffLayoutConfig config;
  config.mode = toLayoutMode(layoutMode_);
  config.rowHeight = rowHeight_;
  config.hunkHeight = hunkHeight_;
  config.fileHeaderHeight = fileHeaderHeight_;
  config.wrapEnabled = wrapEnabled_;

  if (wrapEnabled_) {
    const QFontMetricsF metrics(monoFont(monoFontFamily_, 12));
    const qreal charWidth = metrics.horizontalAdvance('M');
    if (wrapColumn_ > 0) {
      config.unifiedWrapWidth = charWidth * wrapColumn_;
      config.splitWrapWidth = charWidth * wrapColumn_;
    } else if (layoutMode_ == "split") {
      const qreal sideGutter = 22.0 + digitWidth() * (lineNumberDigits_ + 1) + 12.0;
      config.splitWrapWidth = std::max(charWidth * 20.0, (width() - 1.0) / 2.0 - sideGutter - 8.0);
      config.unifiedWrapWidth = std::max(charWidth * 20.0, width() - unifiedGutterWidth() - 24.0);
    } else {
      config.unifiedWrapWidth = std::max(charWidth * 20.0, width() - unifiedGutterWidth() - 24.0);
      config.splitWrapWidth = std::max(charWidth * 20.0, (width() - 1.0) / 2.0);
    }
  }

  displayModel_.rebuild(config);
  contentHeight_ = displayModel_.contentHeight();
  lineNumberDigits_ = displayModel_.lineNumberDigits();
  firstVisibleRow_ = displayModel_.rowIndexAtY(std::max<qreal>(0.0, viewportY_ - hunkHeight_));
  lastVisibleRow_ = displayModel_.rowIndexAtY(viewportY_ + viewportHeight_ + hunkHeight_);
  stickyVisibleRow_ = displayModel_.stickyHunkRowIndexAtY(viewportY_);
  emit displayRowCountChanged();
}

void DiffSurfaceItem::recalculateMetrics() {
  lineNumberDigits_ = displayModel_.lineNumberDigits();
  qreal newContentWidth = 0;
  if (wrapEnabled_) {
    newContentWidth = width();
  } else if (layoutMode_ == "split") {
    const qreal sideGutter = 22.0 + digitWidth() * (lineNumberDigits_ + 1) + 12.0;
    newContentWidth = std::max(width(), maxTextWidth_ + sideGutter + 12.0);
  } else {
    newContentWidth = unifiedGutterWidth() + maxTextWidth_ + 24.0;
  }

  if (!qFuzzyCompare(contentWidth_, newContentWidth)) {
    contentWidth_ = newContentWidth;
    emit contentWidthChanged();
  }

  rebuildDisplayRows();
  emit contentHeightChanged();
  invalidateTiles();
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
  const qreal unifiedBaselineY = wrapEnabled_
      ? rowRect.top() + (rowHeight_ - textMetrics.height()) / 2.0 + textMetrics.ascent()
      : rowRect.top() + (rowRect.height() - textMetrics.height()) / 2.0 + textMetrics.ascent();
  const QRectF textClip(rowRect.left() + gutterWidth + 8.0, rowRect.top(),
                        rowRect.width() - gutterWidth - 12.0, rowRect.height());
  const QColor tokenBg = row.kind == DiffLineKind::Addition ? paletteColor("successBorder", QColor("#38482f"))
                                                            : row.kind == DiffLineKind::Deletion
                                                                  ? paletteColor("dangerBorder", QColor("#4c2b2c"))
                                                                  : paletteColor("accentSoft", QColor("#293b5b"));
  drawTextRun(painter, QPointF(textClip.left(), unifiedBaselineY), textClip, textForRange(row.textRange), row.tokens,
              row.changeSpans, paletteColor("textBase", QColor("#c8ccd4")), tokenBg);
}

void DiffSurfaceItem::drawSplitPaneFixedRow(QPainter* painter,
                                            const QRectF& rowRect,
                                            const DiffDisplayRow& row,
                                            bool isLeftPane,
                                            bool selected) const {
  const qreal sideGutterWidth = 22.0 + digitWidth() * (lineNumberDigits_ + 1) + 12.0;
  const DiffLineKind lineKind = isLeftPane ? row.leftKind : row.rightKind;
  const int lineNumber = isLeftPane ? row.leftLine : row.rightLine;
  const bool spacer = lineKind == DiffLineKind::Spacer;

  const QColor spacerBg = paletteColor("lineContextAlt", QColor("#232323"));
  QColor background = spacerBg;
  if (!spacer) {
    if (isLeftPane && lineKind == DiffLineKind::Deletion) {
      background = paletteColor("lineDelAccent", QColor("#35262b"));
    } else if (!isLeftPane && lineKind == DiffLineKind::Addition) {
      background = paletteColor("lineAddAccent", QColor("#22332a"));
    } else {
      background = paletteColor("lineContext", QColor("#282c33"));
    }
  }

  painter->fillRect(rowRect, background);
  if (selected) {
    painter->fillRect(rowRect, splitSelectionColor(row, isLeftPane));
  }

  painter->fillRect(QRectF(rowRect.left(), rowRect.top(), sideGutterWidth, rowRect.height()),
                    paletteColor("panelTint", QColor("#504945")));
  painter->fillRect(QRectF(rowRect.left() + sideGutterWidth, rowRect.top(), 1.0, rowRect.height()),
                    paletteColor("divider", QColor("#504945")));
  if (isLeftPane) {
    painter->fillRect(QRectF(rowRect.right(), rowRect.top(), 1.0, rowRect.height()),
                      paletteColor("divider", QColor("#363c46")));
  }

  if (isLeftPane && lineKind == DiffLineKind::Deletion) {
    painter->fillRect(QRectF(rowRect.left(), rowRect.top(), 3.0, rowRect.height()),
                      paletteColor("dangerText", QColor("#d07277")));
  }
  if (!isLeftPane && lineKind == DiffLineKind::Addition) {
    painter->fillRect(QRectF(rowRect.left(), rowRect.top(), 3.0, rowRect.height()),
                      paletteColor("successText", QColor("#a1c181")));
  }

  painter->setFont(monoFont(monoFontFamily_, 11));
  painter->setPen(paletteColor("textMuted", QColor("#d5c4a1")));
  painter->drawText(QRectF(rowRect.left() + 6.0, rowRect.top(), 12.0, rowRect.height()), Qt::AlignVCenter,
                    kindSymbol(lineKind));
  const qreal splitNumberWidth = digitWidth() * lineNumberDigits_;
  painter->drawText(QRectF(rowRect.left() + 20.0, rowRect.top(), splitNumberWidth, rowRect.height()),
                    Qt::AlignRight | Qt::AlignVCenter,
                    lineNumber > 0 ? QString::number(lineNumber) : QString());

  if (spacer) {
    QColor guide = paletteColor("divider", QColor("#504945"));
    guide.setAlpha(150);
    painter->fillRect(
        QRectF(rowRect.left() + sideGutterWidth + 8.0, rowRect.top() + 3.0, 1.0, std::max<qreal>(0.0, rowRect.height() - 6.0)),
        guide);
  }
}

void DiffSurfaceItem::drawSplitPaneTextRow(QPainter* painter,
                                           const QRectF& rowRect,
                                           const DiffDisplayRow& row,
                                           bool isLeftPane) const {
  const bool spacer = isLeftPane ? row.leftKind == DiffLineKind::Spacer : row.rightKind == DiffLineKind::Spacer;
  if (spacer) {
    return;
  }

  const TextRange& textRange = isLeftPane ? row.leftTextRange : row.rightTextRange;
  const std::vector<DiffTokenSpan>& tokens = isLeftPane ? row.leftTokens : row.rightTokens;
  const std::vector<DiffTokenSpan>& changeSpans = isLeftPane ? row.leftChangeSpans : row.rightChangeSpans;
  const QFont textFont = monoFont(monoFontFamily_, 12);
  const QFontMetricsF textMetrics(textFont);
  painter->setFont(textFont);
  const qreal baselineY = wrapEnabled_
      ? rowRect.top() + (rowHeight_ - textMetrics.height()) / 2.0 + textMetrics.ascent()
      : rowRect.top() + (rowRect.height() - textMetrics.height()) / 2.0 + textMetrics.ascent();
  drawTextRun(painter, QPointF(rowRect.left(), baselineY), rowRect, textForRange(textRange), tokens, changeSpans,
              paletteColor("textBase", QColor("#c8ccd4")),
              isLeftPane ? paletteColor("dangerBorder", QColor("#4c2b2c"))
                         : paletteColor("successBorder", QColor("#38482f")));
}

void DiffSurfaceItem::drawSplitRow(QPainter* painter,
                                   const QRectF& rowRect,
                                   const DiffDisplayRow& row,
                                   bool selected,
                                   qreal leftViewportX,
                                   qreal rightViewportX) const {
  const QRectF leftRect(rowRect.left(), rowRect.top(), rowRect.width() / 2.0, rowRect.height());
  const QRectF rightRect(leftRect.right(), rowRect.top(), rowRect.width() - leftRect.width(), rowRect.height());
  const qreal sideGutterWidth = 22.0 + digitWidth() * (lineNumberDigits_ + 1) + 12.0;
  const qreal textInset = sideGutterWidth + 8.0;
  const qreal leftTextWidth = std::max<qreal>(0.0, leftRect.width() - sideGutterWidth - 12.0);
  const qreal rightTextWidth = std::max<qreal>(0.0, rightRect.width() - sideGutterWidth - 12.0);
  drawSplitPaneFixedRow(painter, leftRect, row, true, selected);
  drawSplitPaneFixedRow(painter, rightRect, row, false, selected);

  painter->save();
  painter->setClipRect(QRectF(leftRect.left() + textInset, rowRect.top(), leftTextWidth, rowRect.height()));
  painter->translate(leftRect.left() + textInset - leftViewportX, 0.0);
  drawSplitPaneTextRow(painter, QRectF(0.0, rowRect.top(), leftTextWidth, rowRect.height()), row, true);
  painter->restore();

  painter->save();
  painter->setClipRect(QRectF(rightRect.left() + textInset, rowRect.top(), rightTextWidth, rowRect.height()));
  painter->translate(rightRect.left() + textInset - rightViewportX, 0.0);
  drawSplitPaneTextRow(painter, QRectF(0.0, rowRect.top(), rightTextWidth, rowRect.height()), row, false);
  painter->restore();
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

  if (wrapEnabled_) {
    drawTextRunWrapped(painter, baseline, clipRect, text, tokens, changeSpans, textColor, tokenBackground, metrics);
    painter->restore();
    return;
  }

  const auto charX = prefixAdvances(text, metrics);

  for (const DiffTokenSpan& span : changeSpans) {
    const int start = std::max(0, span.start);
    const int end = std::min(static_cast<int>(text.size()), span.start + span.length);
    if (end <= start) {
      continue;
    }
    const qreal startX = baseline.x() + charX[start];
    const qreal spanWidth = charX[end] - charX[start];
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
        painter->drawText(QPointF(baseline.x() + charX[cursor], baseline.y()), text.mid(cursor, tokStart - cursor));
      }
      const QColor fg = syntaxForeground(token.syntaxKind);
      painter->setPen(fg.isValid() ? fg : textColor);
      painter->drawText(QPointF(baseline.x() + charX[tokStart], baseline.y()),
                        text.mid(tokStart, tokEnd - tokStart));
      cursor = tokEnd;
    }
    if (cursor < text.size()) {
      painter->setPen(textColor);
      painter->drawText(QPointF(baseline.x() + charX[cursor], baseline.y()), text.mid(cursor));
    }
  } else {
    painter->setPen(textColor);
    painter->drawText(baseline, text);
  }
  painter->restore();
}

void DiffSurfaceItem::drawTextRunWrapped(QPainter* painter,
                                          const QPointF& baseline,
                                          const QRectF& clipRect,
                                          const QString& text,
                                          const std::vector<DiffTokenSpan>& tokens,
                                          const std::vector<DiffTokenSpan>& changeSpans,
                                          const QColor& textColor,
                                          const QColor& tokenBackground,
                                          const QFontMetricsF& metrics) const {
  const qreal availWidth = clipRect.width();
  const qreal lineH = metrics.height() + 2.0;
  const qreal originX = clipRect.left();
  const auto charX = prefixAdvances(text, metrics);

  auto wrapLine = [&](int charIdx) -> int {
    if (availWidth <= 0) return 0;
    return static_cast<int>(charX[charIdx] / availWidth);
  };

  auto xForChar = [&](int charIdx) -> qreal {
    return originX + charX[charIdx] - wrapLine(charIdx) * availWidth;
  };

  auto yForChar = [&](int charIdx) -> qreal {
    return baseline.y() + wrapLine(charIdx) * lineH;
  };

  for (const DiffTokenSpan& span : changeSpans) {
    const int start = std::max(0, span.start);
    const int end = std::min(static_cast<int>(text.size()), span.start + span.length);
    if (end <= start) continue;

    for (int line = wrapLine(start); line <= wrapLine(end - 1); ++line) {
      int lineStart = start;
      int lineEnd = end;
      for (int c = start; c < end; ++c) {
        if (wrapLine(c) == line) { lineStart = c; break; }
      }
      for (int c = end - 1; c >= start; --c) {
        if (wrapLine(c) == line) { lineEnd = c + 1; break; }
      }
      const qreal sx = xForChar(lineStart);
      const qreal sw = xForChar(lineEnd) - sx;
      const qreal sy = baseline.y() + line * lineH;
      painter->fillRect(QRectF(sx - 1.0, sy - metrics.ascent() - 1.0, sw + 2.0, metrics.height() + 2.0),
                        tokenBackground);
    }
  }

  auto drawSegment = [&](int start, int end, const QColor& color) {
    if (end <= start) return;
    painter->setPen(color);
    int segStart = start;
    while (segStart < end) {
      int segLine = wrapLine(segStart);
      int segEnd = segStart;
      while (segEnd < end && wrapLine(segEnd) == segLine) ++segEnd;
      painter->drawText(QPointF(xForChar(segStart), yForChar(segStart)),
                        text.mid(segStart, segEnd - segStart));
      segStart = segEnd;
    }
  };

  bool hasSyntax = false;
  auto sortedTokens = tokens;
  if (!sortedTokens.empty()) {
    std::sort(sortedTokens.begin(), sortedTokens.end(), [](const DiffTokenSpan& a, const DiffTokenSpan& b) {
      return a.start < b.start;
    });
    for (const auto& t : sortedTokens) {
      if (t.syntaxKind != SyntaxTokenKind::None) { hasSyntax = true; break; }
    }
  }

  if (hasSyntax) {
    int cursor = 0;
    for (const DiffTokenSpan& token : sortedTokens) {
      const int tokStart = std::max(0, token.start);
      const int tokEnd = std::min(static_cast<int>(text.size()), token.start + token.length);
      if (tokEnd <= tokStart) continue;
      if (tokStart > cursor) {
        drawSegment(cursor, tokStart, textColor);
      }
      const QColor fg = syntaxForeground(token.syntaxKind);
      drawSegment(tokStart, tokEnd, fg.isValid() ? fg : textColor);
      cursor = tokEnd;
    }
    if (cursor < text.size()) {
      drawSegment(cursor, text.size(), textColor);
    }
  } else {
    drawSegment(0, text.size(), textColor);
  }
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
  QQuickItem::mousePressEvent(event);
}

void DiffSurfaceItem::mouseMoveEvent(QMouseEvent* event) {
  if (selectionAnchorRow_ >= 0) {
    selectionCursorRow_ = displayModel_.rowIndexAtY(event->position().y() + viewportY_);
    update();
  }
  QQuickItem::mouseMoveEvent(event);
}

void DiffSurfaceItem::mouseReleaseEvent(QMouseEvent* event) {
  if (selectionAnchorRow_ >= 0) {
    selectionCursorRow_ = displayModel_.rowIndexAtY(event->position().y() + viewportY_);
    update();
  }
  QQuickItem::mouseReleaseEvent(event);
}

void DiffSurfaceItem::wheelEvent(QWheelEvent* event) {
  const qreal lineStep = rowHeight_ > 0 ? rowHeight_ : 24.0;
  qreal hPixels = wheelStepPixels(event->pixelDelta().x(), event->angleDelta().x(), lineStep);
  qreal vPixels = wheelStepPixels(event->pixelDelta().y(), event->angleDelta().y(), lineStep);

  if (wrapEnabled_) {
    hPixels = 0.0;
  }

  if ((event->modifiers() & Qt::ShiftModifier) && qFuzzyIsNull(hPixels)) {
    hPixels = vPixels;
    vPixels = 0.0;
  }

  const bool horizontalIntent =
      !qFuzzyIsNull(hPixels) && ((event->modifiers() & Qt::ShiftModifier) || qAbs(hPixels) > qAbs(vPixels));

  if (layoutMode_ == "split" && !wrapEnabled_ && horizontalIntent) {
    const bool isRight = event->position().x() > width() / 2.0;
    if (isRight) {
      setRightViewportX(rightViewportX_ - hPixels);
    } else {
      setLeftViewportX(leftViewportX_ - hPixels);
    }
    event->accept();
    return;
  }

  if (!qFuzzyIsNull(vPixels)) {
    emit scrollToYRequested(viewportY_ - vPixels);
    event->accept();
    return;
  }

  QQuickItem::wheelEvent(event);
}

void DiffSurfaceItem::hoverMoveEvent(QHoverEvent* event) {
  hoveredRow_ = displayModel_.rowIndexAtY(event->position().y() + viewportY_);
  update();
  QQuickItem::hoverMoveEvent(event);
}

void DiffSurfaceItem::hoverLeaveEvent(QHoverEvent* event) {
  hoveredRow_ = -1;
  update();
  QQuickItem::hoverLeaveEvent(event);
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
    QQuickItem::keyPressEvent(event);
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

  if (!wrapEnabled_ && layoutMode_ == "split" && (event->key() == Qt::Key_Left || event->key() == Qt::Key_Right)) {
    const qreal step = event->key() == Qt::Key_Right ? 40.0 : -40.0;
    setLeftViewportX(leftViewportX_ + step);
    setRightViewportX(rightViewportX_ + step);
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

  if (event->key() == Qt::Key_Space && event->modifiers() == Qt::NoModifier) {
    emit scrollToYRequested(viewportY_ + viewportHeight_ * 0.9);
    event->accept();
    return;
  }

  if (event->key() == Qt::Key_Space && event->modifiers() == Qt::ShiftModifier) {
    emit scrollToYRequested(std::max<qreal>(0.0, viewportY_ - viewportHeight_ * 0.9));
    event->accept();
    return;
  }

  if (event->key() == Qt::Key_N && event->modifiers() == Qt::NoModifier) {
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

  if (event->key() == Qt::Key_N && event->modifiers() == Qt::ShiftModifier) {
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

  if (event->key() == Qt::Key_J && event->modifiers() == Qt::NoModifier) {
    emit nextFileRequested();
    event->accept();
    return;
  }

  if (event->key() == Qt::Key_K && event->modifiers() == Qt::NoModifier) {
    emit previousFileRequested();
    event->accept();
    return;
  }

  QQuickItem::keyPressEvent(event);
}

}  // namespace diffy
