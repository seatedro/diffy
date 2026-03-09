#include "app/quick/DiffSurfaceItem.h"

#include <QClipboard>
#include <QColor>
#include <QFont>
#include <QFontMetricsF>
#include <QGuiApplication>
#include <QImage>
#include <QPainter>
#include <QPointer>
#include <QQuickWindow>
#include <QRunnable>
#include <QSGClipNode>
#include <QSGGeometry>
#include <QSGNode>
#include <QSGSimpleRectNode>
#include <QSGSimpleTextureNode>
#include <QStyleHints>
#include <QSet>
#include <QThread>
#include <QTimer>
#include <QThreadPool>
#include <QVector>
#include <QtMath>

#include <algorithm>
#include <chrono>
#include <functional>
#include <limits>

#include "core/syntax/SyntaxTypes.h"

namespace diffy {

struct DiffRasterRow {
  DiffDisplayRow row;
  QString text;
  QString leftText;
  QString rightText;
  std::vector<DiffTokenSpan> tokens;
  std::vector<DiffTokenSpan> changeSpans;
  std::vector<DiffTokenSpan> leftTokens;
  std::vector<DiffTokenSpan> leftChangeSpans;
  std::vector<DiffTokenSpan> rightTokens;
  std::vector<DiffTokenSpan> rightChangeSpans;
};

struct DiffRasterSnapshot {
  quint64 generation = 0;
  QVariantMap palette;
  QString monoFontFamily;
  QString layoutMode;
  bool wrapEnabled = false;
  int wrapColumn = 0;
  qreal rowHeight = 0.0;
  qreal fileHeaderHeight = 0.0;
  qreal hunkHeight = 0.0;
  int lineNumberDigits = 0;
  qreal visibleWidth = 0.0;
  qreal unifiedRowWidth = 0.0;
  qreal splitTextLogicalWidth = 0.0;
  qreal leftPaneWidth = 0.0;
  qreal rightPaneWidth = 0.0;
  qreal leftViewportX = 0.0;
  qreal rightViewportX = 0.0;
  qreal devicePixelRatio = 1.0;
  QHash<int, DiffRasterRow> rows;
};

namespace {

using PerfClock = std::chrono::steady_clock;

double elapsedMs(PerfClock::time_point start) {
  return std::chrono::duration<double, std::milli>(PerfClock::now() - start).count();
}

bool setPerfValue(double& target, double value) {
  if (qAbs(target - value) <= 0.001) {
    return false;
  }
  target = value;
  return true;
}

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

constexpr int kRowTileWidth = 1024;
constexpr int kColumnPrefetchMargin = 1;
constexpr int kMaxResidentTiles = 2048;
constexpr int kMaxRasterTiles = 4096;
constexpr int kMaxLineLayoutCacheEntries = 4096;
constexpr int kMaxWrappedLayoutCacheEntries = 4096;
constexpr int kTilePrewarmBatchSize = 12;
constexpr int kTilePrewarmRowMargin = 12;
constexpr int kSyncRasterFallbackTileBudget = 60;
constexpr double kSyncRasterFallbackMsBudget = 8.0;
constexpr int kViewportSyncRasterFallbackTileBudget = 60;
constexpr double kViewportSyncRasterFallbackMsBudget = 8.0;
constexpr int kCurrentViewportRasterPriority = 2;
constexpr int kVisibleTileRequestPriority = 3;
constexpr int kAlternatePrewarmRasterPriority = -1;

quint64 tileKey(quint64 contentGeneration,
                quint64 geometryGeneration,
                quint64 paletteGeneration,
                TileLayer layer,
                int rowIndex,
                int columnIndex) {
  quint64 hash = qHashMulti(0u, contentGeneration, geometryGeneration, paletteGeneration);
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

class LayerClipNode final : public QSGClipNode {
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

QColor splitPaneBackgroundColor(const DiffDisplayRow& row, bool isLeftPane) {
  const DiffLineKind lineKind = isLeftPane ? row.leftKind : row.rightKind;
  if (lineKind == DiffLineKind::Spacer) {
    return QColor("#232323");
  }
  if (isLeftPane && lineKind == DiffLineKind::Deletion) {
    return QColor("#35262b");
  }
  if (!isLeftPane && lineKind == DiffLineKind::Addition) {
    return QColor("#22332a");
  }
  return QColor("#282c33");
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

qint64 wrapWidthCacheKey(qreal wrapWidth) {
  if (wrapWidth <= 0.0) {
    return -1;
  }
  return qRound64(wrapWidth * 1000.0);
}

QColor snapshotPaletteColor(const DiffRasterSnapshot& snapshot, const char* key, const QColor& fallback) {
  const QVariant value = snapshot.palette.value(QString::fromLatin1(key));
  if (!value.isValid()) {
    return fallback;
  }
  const QColor color = value.value<QColor>();
  return color.isValid() ? color : fallback;
}

struct SnapshotLineLayout {
  qreal width = 0.0;
  std::vector<qreal> prefixAdvances;
};

struct SnapshotWrappedLayout {
  int lineCount = 1;
  std::vector<int> charWrapLines;
};

class SnapshotRenderer {
 public:
  explicit SnapshotRenderer(const DiffRasterSnapshot& snapshot)
      : snapshot_(snapshot),
        font11_(monoFont(snapshot_.monoFontFamily, 11)),
        font12_(monoFont(snapshot_.monoFontFamily, 12)),
        metrics11_(font11_),
        metrics12_(font12_),
        charWidth11_(metrics11_.horizontalAdvance(QLatin1Char('M'))),
        charWidth12_(metrics12_.horizontalAdvance(QLatin1Char('M'))) {}

  QImage renderTileImage(const TileSpec& spec) const {
    Q_ASSERT(spec.rowIndex >= 0);
    const auto it = snapshot_.rows.constFind(spec.rowIndex);
    Q_ASSERT(it != snapshot_.rows.constEnd());
    const DiffRasterRow& row = it.value();
    const QSize pixelSize(qMax(1, qCeil(spec.targetRect.width() * snapshot_.devicePixelRatio)),
                          qMax(1, qCeil(row.row.height * snapshot_.devicePixelRatio)));
    QImage image(pixelSize, QImage::Format_ARGB32_Premultiplied);
    image.setDevicePixelRatio(snapshot_.devicePixelRatio);
    image.fill(snapshotPaletteColor(snapshot_, "canvas", QColor("#282828")).rgb());

    QPainter painter(&image);
    painter.setRenderHint(QPainter::TextAntialiasing, true);
    painter.setRenderHint(QPainter::Antialiasing, false);
    painter.translate(-spec.logicalX, -row.row.top);

    switch (spec.layer) {
      case TileLayer::UnifiedRow: {
        const QRectF rowRect(0.0, row.row.top, snapshot_.unifiedRowWidth, row.row.height);
        if (row.row.rowType == DiffRowType::FileHeader) {
          drawFileHeaderRow(&painter, rowRect, row);
        } else if (row.row.rowType == DiffRowType::Hunk) {
          drawHunkRow(&painter, rowRect, row);
        } else {
          drawUnifiedRow(&painter, rowRect, row);
        }
        break;
      }
      case TileLayer::SplitFullRow: {
        const QRectF rowRect(0.0, row.row.top, snapshot_.visibleWidth, row.row.height);
        if (row.row.rowType == DiffRowType::FileHeader) {
          drawFileHeaderRow(&painter, rowRect, row);
        } else if (row.row.rowType == DiffRowType::Hunk) {
          drawHunkRow(&painter, rowRect, row);
        } else {
          drawSplitRow(&painter, rowRect, row, snapshot_.leftViewportX, snapshot_.rightViewportX);
        }
        break;
      }
      case TileLayer::SplitLeftFixedRow:
      case TileLayer::SplitRightFixedRow: {
        const bool isLeftPane = spec.layer == TileLayer::SplitLeftFixedRow;
        const qreal paneWidth = isLeftPane ? snapshot_.leftPaneWidth : snapshot_.rightPaneWidth;
        drawSplitPaneFixedRow(&painter, QRectF(0.0, row.row.top, paneWidth, row.row.height), row, isLeftPane);
        break;
      }
      case TileLayer::SplitLeftTextRow:
      case TileLayer::SplitRightTextRow: {
        const bool isLeftPane = spec.layer == TileLayer::SplitLeftTextRow;
        drawSplitPaneTextRow(&painter, QRectF(0.0, row.row.top, snapshot_.splitTextLogicalWidth, row.row.height), row,
                             isLeftPane);
        break;
      }
      case TileLayer::StickyRow: {
        const QRectF rowRect(0.0, row.row.top, spec.targetRect.width(), row.row.height);
        if (row.row.rowType == DiffRowType::FileHeader) {
          drawFileHeaderRow(&painter, rowRect, row);
        } else if (row.row.rowType == DiffRowType::Hunk) {
          drawHunkRow(&painter, rowRect, row);
        }
        break;
      }
    }

    painter.end();
    return image;
  }

 private:
  qreal digitWidth() const {
    return charWidth11_;
  }

  qreal unifiedGutterWidth() const {
    return 12.0 + 14.0 + digitWidth() * (snapshot_.lineNumberDigits * 2 + 2) + 28.0;
  }

  SnapshotLineLayout lineLayoutForText(const QString& text, int pixelSize) const {
    SnapshotLineLayout layout;
    const qreal charWidth = pixelSize <= 11 ? charWidth11_ : charWidth12_;
    layout.prefixAdvances.reserve(static_cast<size_t>(text.size() + 1));
    layout.prefixAdvances.push_back(0.0);
    for (int i = 0; i < text.size(); ++i) {
      layout.prefixAdvances.push_back(charWidth * (i + 1));
    }
    layout.width = text.isEmpty() ? 0.0 : charWidth * text.size();
    return layout;
  }

  SnapshotWrappedLayout wrappedLayoutForText(const QString& text, int pixelSize, qreal wrapWidth) const {
    SnapshotWrappedLayout wrappedLayout;
    const qreal charWidth = pixelSize <= 11 ? charWidth11_ : charWidth12_;
    wrappedLayout.charWrapLines.resize(static_cast<size_t>(text.size() + 1), 0);
    if (wrapWidth > 0.0 && !text.isEmpty()) {
      int currentLine = 0;
      qreal nextBoundary = wrapWidth;
      for (int i = 0; i <= text.size(); ++i) {
        const qreal advance = charWidth * i;
        while (advance >= nextBoundary) {
          ++currentLine;
          nextBoundary += wrapWidth;
        }
        wrappedLayout.charWrapLines[i] = currentLine;
      }
      wrappedLayout.lineCount = currentLine + 1;
    }
    return wrappedLayout;
  }

  void drawFileHeaderRow(QPainter* painter, const QRectF& rowRect, const DiffRasterRow& row) const {
    painter->fillRect(rowRect, snapshotPaletteColor(snapshot_, "canvas", QColor("#282828")));
    painter->fillRect(QRectF(rowRect.left(), rowRect.bottom() - 1.0, rowRect.width(), 1.0),
                      snapshotPaletteColor(snapshot_, "divider", QColor("#363c46")));

    painter->setFont(font11_);
    painter->setPen(snapshotPaletteColor(snapshot_, "textStrong", QColor("#fbf1c7")));
    painter->drawText(QRectF(rowRect.left() + 12.0, rowRect.top(), rowRect.width() - 160.0, rowRect.height()),
                      Qt::AlignVCenter | Qt::AlignLeft, QString::fromStdString(row.row.header));

    painter->setPen(snapshotPaletteColor(snapshot_, "textMuted", QColor("#d5c4a1")));
    painter->drawText(QRectF(rowRect.right() - 140.0, rowRect.top(), 130.0, rowRect.height()),
                      Qt::AlignVCenter | Qt::AlignRight, QString::fromStdString(row.row.detail));
  }

  void drawHunkRow(QPainter* painter, const QRectF& rowRect, const DiffRasterRow& row) const {
    painter->fillRect(QRectF(rowRect.left(), rowRect.top(), rowRect.width(), 1.0),
                      snapshotPaletteColor(snapshot_, "divider", QColor("#363c46")));
    painter->fillRect(QRectF(rowRect.left(), rowRect.top() + 1.0, rowRect.width(), rowRect.height() - 2.0),
                      snapshotPaletteColor(snapshot_, "panelStrong", QColor("#3b414d")));
    painter->fillRect(QRectF(rowRect.left(), rowRect.bottom() - 1.0, rowRect.width(), 1.0),
                      snapshotPaletteColor(snapshot_, "divider", QColor("#363c46")));

    painter->setFont(font11_);
    painter->setPen(snapshotPaletteColor(snapshot_, "textMuted", QColor("#a9afbc")));
    painter->drawText(QRectF(rowRect.left() + 10.0, rowRect.top(), rowRect.width() - 20.0, rowRect.height()),
                      Qt::AlignVCenter | Qt::AlignLeft, QString::fromStdString(row.row.header));
  }

  void drawUnifiedRow(QPainter* painter, const QRectF& rowRect, const DiffRasterRow& row) const {
    QColor background = snapshotPaletteColor(snapshot_, "lineContext", QColor("#282c33"));
    if (row.row.kind == DiffLineKind::Addition) {
      background = snapshotPaletteColor(snapshot_, "lineAdd", QColor("#1f2d24"));
    } else if (row.row.kind == DiffLineKind::Deletion) {
      background = snapshotPaletteColor(snapshot_, "lineDel", QColor("#2d2024"));
    }

    painter->fillRect(rowRect, background);

    const qreal gutterWidth = unifiedGutterWidth();
    const QRectF gutterRect(rowRect.left(), rowRect.top(), gutterWidth, rowRect.height());
    painter->fillRect(gutterRect, snapshotPaletteColor(snapshot_, "panelTint", QColor("#353b45")));
    painter->fillRect(QRectF(gutterRect.right(), rowRect.top(), 1.0, rowRect.height()),
                      snapshotPaletteColor(snapshot_, "divider", QColor("#363c46")));

    if (row.row.kind == DiffLineKind::Addition || row.row.kind == DiffLineKind::Deletion) {
      const QColor marker = row.row.kind == DiffLineKind::Addition
                                ? snapshotPaletteColor(snapshot_, "successText", QColor("#a1c181"))
                                : snapshotPaletteColor(snapshot_, "dangerText", QColor("#d07277"));
      painter->fillRect(QRectF(rowRect.left(), rowRect.top(), 3.0, rowRect.height()), marker);
    }

    painter->setFont(font11_);
    painter->setPen(row.row.kind == DiffLineKind::Addition
                        ? snapshotPaletteColor(snapshot_, "successText", QColor("#a1c181"))
                        : row.row.kind == DiffLineKind::Deletion
                              ? snapshotPaletteColor(snapshot_, "dangerText", QColor("#d07277"))
                              : snapshotPaletteColor(snapshot_, "textMuted", QColor("#a9afbc")));
    painter->drawText(QRectF(rowRect.left() + 6.0, rowRect.top(), 12.0, rowRect.height()), Qt::AlignVCenter,
                      kindSymbol(row.row.kind));

    painter->setPen(snapshotPaletteColor(snapshot_, "textMuted", QColor("#d5c4a1")));
    const qreal numberWidth = digitWidth() * snapshot_.lineNumberDigits;
    painter->drawText(QRectF(rowRect.left() + 22.0, rowRect.top(), numberWidth, rowRect.height()),
                      Qt::AlignRight | Qt::AlignVCenter,
                      row.row.oldLine > 0 ? QString::number(row.row.oldLine) : QString());

    painter->fillRect(QRectF(rowRect.left() + 26.0 + numberWidth, rowRect.top() + 4.0, 1.0, rowRect.height() - 8.0),
                      snapshotPaletteColor(snapshot_, "divider", QColor("#504945")));

    painter->drawText(QRectF(rowRect.left() + 32.0 + numberWidth, rowRect.top(), numberWidth, rowRect.height()),
                      Qt::AlignRight | Qt::AlignVCenter,
                      row.row.newLine > 0 ? QString::number(row.row.newLine) : QString());

    painter->setFont(font12_);
    const qreal baselineY = snapshot_.wrapEnabled
                                ? rowRect.top() + (snapshot_.rowHeight - metrics12_.height()) / 2.0 + metrics12_.ascent()
                                : rowRect.top() + (rowRect.height() - metrics12_.height()) / 2.0 + metrics12_.ascent();
    const QRectF textClip(rowRect.left() + gutterWidth + 8.0, rowRect.top(), rowRect.width() - gutterWidth - 12.0,
                          rowRect.height());
    const QColor tokenBg = row.row.kind == DiffLineKind::Addition
                               ? snapshotPaletteColor(snapshot_, "successBorder", QColor("#38482f"))
                               : row.row.kind == DiffLineKind::Deletion
                                     ? snapshotPaletteColor(snapshot_, "dangerBorder", QColor("#4c2b2c"))
                                     : snapshotPaletteColor(snapshot_, "accentSoft", QColor("#293b5b"));
    const SnapshotLineLayout layout = lineLayoutForText(row.text, 12);
    drawTextRun(painter, QPointF(textClip.left(), baselineY), textClip, row.text, row.tokens.data(), row.tokens.size(),
                row.changeSpans.data(), row.changeSpans.size(), layout.prefixAdvances,
                snapshotPaletteColor(snapshot_, "textBase", QColor("#c8ccd4")), tokenBg);
  }

  void drawSplitPaneFixedRow(QPainter* painter, const QRectF& rowRect, const DiffRasterRow& row, bool isLeftPane) const {
    const qreal sideGutterWidth = 22.0 + digitWidth() * (snapshot_.lineNumberDigits + 1) + 12.0;
    const DiffLineKind lineKind = isLeftPane ? row.row.leftKind : row.row.rightKind;
    const int lineNumber = isLeftPane ? row.row.leftLine : row.row.rightLine;
    const bool spacer = lineKind == DiffLineKind::Spacer;

    QColor background = splitPaneBackgroundColor(row.row, isLeftPane);
    if (spacer) {
      background = snapshotPaletteColor(snapshot_, "lineContextAlt", background);
    } else if (isLeftPane && lineKind == DiffLineKind::Deletion) {
      background = snapshotPaletteColor(snapshot_, "lineDelAccent", background);
    } else if (!isLeftPane && lineKind == DiffLineKind::Addition) {
      background = snapshotPaletteColor(snapshot_, "lineAddAccent", background);
    } else {
      background = snapshotPaletteColor(snapshot_, "lineContext", background);
    }

    painter->fillRect(rowRect, background);
    painter->fillRect(QRectF(rowRect.left(), rowRect.top(), sideGutterWidth, rowRect.height()),
                      snapshotPaletteColor(snapshot_, "panelTint", QColor("#504945")));
    painter->fillRect(QRectF(rowRect.left() + sideGutterWidth, rowRect.top(), 1.0, rowRect.height()),
                      snapshotPaletteColor(snapshot_, "divider", QColor("#504945")));
    if (isLeftPane) {
      painter->fillRect(QRectF(rowRect.right(), rowRect.top(), 1.0, rowRect.height()),
                        snapshotPaletteColor(snapshot_, "divider", QColor("#363c46")));
    }

    if (isLeftPane && lineKind == DiffLineKind::Deletion) {
      painter->fillRect(QRectF(rowRect.left(), rowRect.top(), 3.0, rowRect.height()),
                        snapshotPaletteColor(snapshot_, "dangerText", QColor("#d07277")));
    }
    if (!isLeftPane && lineKind == DiffLineKind::Addition) {
      painter->fillRect(QRectF(rowRect.left(), rowRect.top(), 3.0, rowRect.height()),
                        snapshotPaletteColor(snapshot_, "successText", QColor("#a1c181")));
    }

    painter->setFont(font11_);
    painter->setPen(snapshotPaletteColor(snapshot_, "textMuted", QColor("#d5c4a1")));
    painter->drawText(QRectF(rowRect.left() + 6.0, rowRect.top(), 12.0, rowRect.height()), Qt::AlignVCenter,
                      kindSymbol(lineKind));
    const qreal splitNumberWidth = digitWidth() * snapshot_.lineNumberDigits;
    painter->drawText(QRectF(rowRect.left() + 20.0, rowRect.top(), splitNumberWidth, rowRect.height()),
                      Qt::AlignRight | Qt::AlignVCenter, lineNumber > 0 ? QString::number(lineNumber) : QString());

    if (spacer) {
      QColor guide = snapshotPaletteColor(snapshot_, "divider", QColor("#504945"));
      guide.setAlpha(150);
      painter->fillRect(QRectF(rowRect.left() + sideGutterWidth + 8.0, rowRect.top() + 3.0, 1.0,
                               std::max<qreal>(0.0, rowRect.height() - 6.0)),
                        guide);
    }
  }

  void drawSplitPaneTextRow(QPainter* painter, const QRectF& rowRect, const DiffRasterRow& row, bool isLeftPane) const {
    const DiffLineKind lineKind = isLeftPane ? row.row.leftKind : row.row.rightKind;
    QColor background = splitPaneBackgroundColor(row.row, isLeftPane);
    if (lineKind == DiffLineKind::Spacer) {
      background = snapshotPaletteColor(snapshot_, "lineContextAlt", background);
    } else if (isLeftPane && lineKind == DiffLineKind::Deletion) {
      background = snapshotPaletteColor(snapshot_, "lineDelAccent", background);
    } else if (!isLeftPane && lineKind == DiffLineKind::Addition) {
      background = snapshotPaletteColor(snapshot_, "lineAddAccent", background);
    } else {
      background = snapshotPaletteColor(snapshot_, "lineContext", background);
    }
    painter->fillRect(rowRect, background);

    if (lineKind == DiffLineKind::Spacer) {
      return;
    }

    const QString& text = isLeftPane ? row.leftText : row.rightText;
    const std::vector<DiffTokenSpan>& tokens = isLeftPane ? row.leftTokens : row.rightTokens;
    const std::vector<DiffTokenSpan>& changeSpans =
        isLeftPane ? row.leftChangeSpans : row.rightChangeSpans;
    const SnapshotLineLayout layout = lineLayoutForText(text, 12);
    painter->setFont(font12_);
    const qreal baselineY = snapshot_.wrapEnabled
                                ? rowRect.top() + (snapshot_.rowHeight - metrics12_.height()) / 2.0 + metrics12_.ascent()
                                : rowRect.top() + (rowRect.height() - metrics12_.height()) / 2.0 + metrics12_.ascent();
    drawTextRun(painter, QPointF(rowRect.left(), baselineY), rowRect, text, tokens.data(), tokens.size(),
                changeSpans.data(), changeSpans.size(), layout.prefixAdvances,
                snapshotPaletteColor(snapshot_, "textBase", QColor("#c8ccd4")),
                isLeftPane ? snapshotPaletteColor(snapshot_, "dangerBorder", QColor("#4c2b2c"))
                           : snapshotPaletteColor(snapshot_, "successBorder", QColor("#38482f")));
  }

  void drawSplitRow(QPainter* painter,
                    const QRectF& rowRect,
                    const DiffRasterRow& row,
                    qreal leftViewportX,
                    qreal rightViewportX) const {
    const QRectF leftRect(rowRect.left(), rowRect.top(), rowRect.width() / 2.0, rowRect.height());
    const QRectF rightRect(leftRect.right(), rowRect.top(), rowRect.width() - leftRect.width(), rowRect.height());
    const qreal sideGutterWidth = 22.0 + digitWidth() * (snapshot_.lineNumberDigits + 1) + 12.0;
    const qreal textInset = sideGutterWidth + 8.0;
    const qreal leftTextWidth = std::max<qreal>(0.0, leftRect.width() - sideGutterWidth - 12.0);
    const qreal rightTextWidth = std::max<qreal>(0.0, rightRect.width() - sideGutterWidth - 12.0);
    drawSplitPaneFixedRow(painter, leftRect, row, true);
    drawSplitPaneFixedRow(painter, rightRect, row, false);

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

  void drawTextRun(QPainter* painter,
                   const QPointF& baseline,
                   const QRectF& clipRect,
                   const QString& text,
                   const DiffTokenSpan* tokens,
                   size_t tokenCount,
                   const DiffTokenSpan* changeSpans,
                   size_t changeSpanCount,
                   const std::vector<qreal>& charX,
                   const QColor& textColor,
                   const QColor& tokenBackground) const {
    painter->save();
    painter->setClipRect(clipRect);

    painter->setFont(font12_);

    if (snapshot_.wrapEnabled) {
      drawTextRunWrapped(painter, baseline, clipRect, text, tokens, tokenCount, changeSpans, changeSpanCount, charX,
                         textColor, tokenBackground, metrics12_);
      painter->restore();
      return;
    }

    for (size_t i = 0; i < changeSpanCount; ++i) {
      const DiffTokenSpan& span = changeSpans[i];
      const int start = std::max(0, span.start);
      const int end = std::min(static_cast<int>(text.size()), span.start + span.length);
      if (end <= start) {
        continue;
      }
      const qreal startX = baseline.x() + charX[start];
      const qreal spanWidth = charX[end] - charX[start];
      painter->fillRect(QRectF(startX - 1.0, baseline.y() - metrics12_.ascent() - 1.0, spanWidth + 2.0,
                               metrics12_.height() + 2.0),
                        tokenBackground);
    }

    bool hasSyntax = false;
    std::vector<DiffTokenSpan> sortedTokens(tokens, tokens + tokenCount);
    if (!sortedTokens.empty()) {
      std::sort(sortedTokens.begin(), sortedTokens.end(), [](const DiffTokenSpan& lhs, const DiffTokenSpan& rhs) {
        return lhs.start < rhs.start;
      });
      for (const auto& token : sortedTokens) {
        if (token.syntaxKind != SyntaxTokenKind::None) {
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
        painter->drawText(QPointF(baseline.x() + charX[tokStart], baseline.y()), text.mid(tokStart, tokEnd - tokStart));
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

  void drawTextRunWrapped(QPainter* painter,
                          const QPointF& baseline,
                          const QRectF& clipRect,
                          const QString& text,
                          const DiffTokenSpan* tokens,
                          size_t tokenCount,
                          const DiffTokenSpan* changeSpans,
                          size_t changeSpanCount,
                          const std::vector<qreal>& charX,
                          const QColor& textColor,
                          const QColor& tokenBackground,
                          const QFontMetricsF& metrics) const {
    const auto wrappedLayout = wrappedLayoutForText(text, 12, clipRect.width());
    const qreal lineH = metrics.height() + 2.0;
    const qreal originX = clipRect.left();

    auto wrapLine = [&](int charIdx) -> int {
      if (wrappedLayout.charWrapLines.empty()) {
        return 0;
      }
      const int clampedIndex = std::clamp(charIdx, 0, static_cast<int>(wrappedLayout.charWrapLines.size()) - 1);
      return wrappedLayout.charWrapLines[clampedIndex];
    };

    auto xForChar = [&](int charIdx) -> qreal {
      const qreal availWidth = clipRect.width();
      return originX + charX[charIdx] - wrapLine(charIdx) * availWidth;
    };

    auto yForChar = [&](int charIdx) -> qreal { return baseline.y() + wrapLine(charIdx) * lineH; };

    for (size_t i = 0; i < changeSpanCount; ++i) {
      const DiffTokenSpan& span = changeSpans[i];
      const int start = std::max(0, span.start);
      const int end = std::min(static_cast<int>(text.size()), span.start + span.length);
      if (end <= start) {
        continue;
      }

      for (int line = wrapLine(start); line <= wrapLine(end - 1); ++line) {
        int lineStart = start;
        int lineEnd = end;
        for (int c = start; c < end; ++c) {
          if (wrapLine(c) == line) {
            lineStart = c;
            break;
          }
        }
        for (int c = end - 1; c >= start; --c) {
          if (wrapLine(c) == line) {
            lineEnd = c + 1;
            break;
          }
        }
        const qreal sx = xForChar(lineStart);
        const qreal sw = xForChar(lineEnd) - sx;
        const qreal sy = baseline.y() + line * lineH;
        painter->fillRect(QRectF(sx - 1.0, sy - metrics.ascent() - 1.0, sw + 2.0, metrics.height() + 2.0),
                          tokenBackground);
      }
    }

    auto drawSegment = [&](int start, int end, const QColor& color) {
      if (end <= start) {
        return;
      }
      painter->setPen(color);
      int segStart = start;
      while (segStart < end) {
        const int segLine = wrapLine(segStart);
        int segEnd = segStart;
        while (segEnd < end && wrapLine(segEnd) == segLine) {
          ++segEnd;
        }
        painter->drawText(QPointF(xForChar(segStart), yForChar(segStart)), text.mid(segStart, segEnd - segStart));
        segStart = segEnd;
      }
    };

    bool hasSyntax = false;
    std::vector<DiffTokenSpan> sortedTokens(tokens, tokens + tokenCount);
    if (!sortedTokens.empty()) {
      std::sort(sortedTokens.begin(), sortedTokens.end(), [](const DiffTokenSpan& lhs, const DiffTokenSpan& rhs) {
        return lhs.start < rhs.start;
      });
      for (const auto& token : sortedTokens) {
        if (token.syntaxKind != SyntaxTokenKind::None) {
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

  const DiffRasterSnapshot& snapshot_;
  const QFont font11_;
  const QFont font12_;
  const QFontMetricsF metrics11_;
  const QFontMetricsF metrics12_;
  const qreal charWidth11_;
  const qreal charWidth12_;
};

class RasterTileJob final : public QRunnable {
 public:
  using OnReady = std::function<void(QImage)>;

  RasterTileJob(std::shared_ptr<const DiffRasterSnapshot> snapshot, TileSpec spec, OnReady onReady)
      : snapshot_(std::move(snapshot)), spec_(std::move(spec)), onReady_(std::move(onReady)) {}

  void run() override {
    QImage image = SnapshotRenderer(*snapshot_).renderTileImage(spec_);
    onReady_(std::move(image));
  }

 private:
  std::shared_ptr<const DiffRasterSnapshot> snapshot_;
  TileSpec spec_;
  OnReady onReady_;
};

}  // namespace

DiffSurfaceItem::DiffSurfaceItem(QQuickItem* parent) : QQuickItem(parent) {
  setFlag(ItemHasContents, true);
  setFlag(ItemObservesViewport, true);
  setAcceptedMouseButtons(Qt::LeftButton);
  setAcceptHoverEvents(true);
  setFocus(true);
  rasterThreadPool_.setMaxThreadCount(std::max(1, QThread::idealThreadCount() - 1));
  rasterThreadPool_.setExpiryTimeout(5000);
  connect(this, &QQuickItem::widthChanged, this, [this]() {
    invalidateRasterJobs();
    scheduleMetricsRecalc();
  });
  connect(this, &QQuickItem::heightChanged, this, [this]() {
    invalidateRasterJobs();
    update();
  });
}

QObject* DiffSurfaceItem::rowsModel() const {
  return rowsModelObject_;
}

void DiffSurfaceItem::invalidateContentTiles() {
  update();
  updateTileStats();
}

void DiffSurfaceItem::invalidateGeometryTiles() {
  update();
  updateTileStats();
}

void DiffSurfaceItem::invalidatePaletteTiles() {
  ++tilePaletteGeneration_;
  update();
  updateTileStats();
}

void DiffSurfaceItem::updateTileStats() {
  emit tileStatsChanged();
}

bool DiffSurfaceItem::updateFileHeader() {
  const bool hadHeader = displayModel_.fileHeaderRowIndex() >= 0;
  const int lineNumberDigitsBefore = displayModel_.lineNumberDigits();
  if (filePath_.isEmpty()) {
    displayModel_.setFileHeader(std::nullopt);
    return hadHeader || lineNumberDigitsBefore != displayModel_.lineNumberDigits();
  }

  DiffSourceRow headerRow;
  headerRow.rowType = DiffRowType::FileHeader;
  headerRow.header = filePath_.toStdString();
  headerRow.detail = fileStatus_.toStdString() + " +" + std::to_string(additions_) + " -" +
                     std::to_string(deletions_);
  displayModel_.setFileHeader(std::move(headerRow));
  return !hadHeader || lineNumberDigitsBefore != displayModel_.lineNumberDigits();
}

void DiffSurfaceItem::scheduleRowsRebuild() {
  if (rowsRebuildQueued_) {
    return;
  }
  viewportJumpFallbackArmed_ = false;
  invalidateRasterJobs(true);
  rowsRebuildQueued_ = true;
  QTimer::singleShot(0, this, [this]() {
    rowsRebuildQueued_ = false;
    metricsRecalcQueued_ = false;
    rebuildRows();
  });
}

void DiffSurfaceItem::scheduleMetricsRecalc() {
  if (rowsRebuildQueued_ || metricsRecalcQueued_) {
    return;
  }
  metricsRecalcQueued_ = true;
  QTimer::singleShot(0, this, [this]() {
    metricsRecalcQueued_ = false;
    if (!rowsRebuildQueued_) {
      recalculateMetrics();
    }
  });
}

void DiffSurfaceItem::scheduleAlternateLayoutPrewarm() {
  if (alternateLayoutPrewarmQueued_ || rowsModel_ == nullptr || rowsModel_->rows().empty() ||
      rowsRebuildQueued_ || metricsRecalcQueued_ || width() <= 0.0) {
    return;
  }

  alternateLayoutPrewarmQueued_ = true;
  QTimer::singleShot(0, this, [this]() {
    alternateLayoutPrewarmQueued_ = false;
    if (rowsModel_ == nullptr || rowsModel_->rows().empty() || rowsRebuildQueued_ || metricsRecalcQueued_ ||
        width() <= 0.0) {
      return;
    }

    const QString alternateMode = layoutMode_ == "split" ? QStringLiteral("unified") : QStringLiteral("split");
    displayModel_.prewarm(buildLayoutConfig(alternateMode));
    scheduleAlternateTilePrewarm();
  });
}

void DiffSurfaceItem::scheduleCurrentTileRaster() {
  if (currentTileRasterQueued_ || rowsModel_ == nullptr || rowsModel_->rows().empty() || rowsRebuildQueued_ ||
      metricsRecalcQueued_ || width() <= 0.0 || height() <= 0.0) {
    return;
  }

  currentTileRasterQueued_ = true;
  QTimer::singleShot(0, this, [this]() {
    currentTileRasterQueued_ = false;
    if (rowsModel_ == nullptr || rowsModel_->rows().empty() || rowsRebuildQueued_ || metricsRecalcQueued_ ||
        width() <= 0.0 || height() <= 0.0) {
      return;
    }
    dispatchTileRaster(layoutMode_, kCurrentViewportRasterPriority);
  });
}

DiffLayoutConfig DiffSurfaceItem::buildLayoutConfig(const QString& mode) const {
  DiffLayoutConfig config;
  config.mode = toLayoutMode(mode);
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
    } else if (mode == "split") {
      const qreal sideGutter = 22.0 + digitWidth() * (lineNumberDigits_ + 1) + 12.0;
      config.splitWrapWidth = std::max(charWidth * 20.0, (width() - 1.0) / 2.0 - sideGutter - 8.0);
      config.unifiedWrapWidth = std::max(charWidth * 20.0, width() - unifiedGutterWidth() - 24.0);
    } else {
      config.unifiedWrapWidth = std::max(charWidth * 20.0, width() - unifiedGutterWidth() - 24.0);
      config.splitWrapWidth = std::max(charWidth * 20.0, (width() - 1.0) / 2.0);
    }
  }

  return config;
}

qreal DiffSurfaceItem::contentWidthForLayout(const QString& mode) const {
  if (wrapEnabled_) {
    return width();
  }
  if (mode == "split") {
    const qreal sideGutter = 22.0 + digitWidth() * (lineNumberDigits_ + 1) + 12.0;
    return std::max(width(), maxTextWidth_ + sideGutter + 12.0);
  }
  return unifiedGutterWidth() + maxTextWidth_ + 24.0;
}

QImage DiffSurfaceItem::renderTileImageInline(const std::vector<DiffDisplayRow>& rows,
                                              const TileSpec& spec,
                                              qreal visibleWidth,
                                              qreal unifiedRowWidth,
                                              qreal splitTextLogicalWidth,
                                              qreal leftPaneWidth,
                                              qreal rightPaneWidth,
                                              qreal devicePixelRatio) const {
  Q_ASSERT(spec.rowIndex >= 0);
  Q_ASSERT(spec.rowIndex < static_cast<int>(rows.size()));
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
}

std::vector<TileSpec> DiffSurfaceItem::buildPrewarmTileSpecs(const QString& mode) {
  if (width() <= 0.0 || height() <= 0.0) {
    return {};
  }

  const DiffLayoutConfig config = buildLayoutConfig(mode);
  const auto& rows = displayModel_.cachedRows(config);
  if (rows.empty()) {
    return {};
  }

  const qreal visibleWidth = width();
  const qreal visibleHeight = height();
  const qreal contentWidth = contentWidthForLayout(mode);
  const qreal unifiedRowWidth = std::max(visibleWidth, contentWidth);
  const qreal leftPaneWidth = visibleWidth / 2.0;
  const qreal rightPaneWidth = visibleWidth - leftPaneWidth;
  const qreal sideGutterWidth = 22.0 + digitWidth() * (lineNumberDigits_ + 1) + 12.0;
  const qreal splitTextInset = sideGutterWidth + 8.0;
  const qreal leftTextViewportWidth = std::max<qreal>(0.0, leftPaneWidth - splitTextInset - 12.0);
  const qreal rightTextViewportWidth = std::max<qreal>(0.0, rightPaneWidth - splitTextInset - 12.0);
  const qreal splitTextLogicalWidth = std::max({maxTextWidth_ + 12.0, leftTextViewportWidth, rightTextViewportWidth});
  const quint64 currentTileContentKey = tileContentKey();
  const quint64 currentTileGeometryKey =
      tileGeometryKey(mode, contentWidth, visibleWidth, visibleHeight, unifiedRowWidth, splitTextLogicalWidth);

  auto keyForTile = [&](TileLayer layer, int rowIndex, int columnIndex) {
    return tileKey(currentTileContentKey, currentTileGeometryKey, tilePaletteGeneration_, layer, rowIndex, columnIndex);
  };

  std::vector<int> orderedRows;
  orderedRows.reserve(rows.size());
  const int firstRow = std::clamp(
      displayModel_.rowIndexAtY(config, std::max<qreal>(0.0, viewportY_ - hunkHeight_)), 0, static_cast<int>(rows.size()) - 1);
  const int lastRow = std::clamp(
      displayModel_.rowIndexAtY(config, viewportY_ + viewportHeight_ + hunkHeight_), firstRow, static_cast<int>(rows.size()) - 1);
  QSet<int> addedRows;
  auto appendRow = [&](int rowIndex) {
    if (rowIndex < 0 || rowIndex >= static_cast<int>(rows.size()) || addedRows.contains(rowIndex)) {
      return;
    }
    orderedRows.push_back(rowIndex);
    addedRows.insert(rowIndex);
  };

  const int stickyIndex = displayModel_.stickyHunkRowIndexAtY(config, viewportY_);
  appendRow(stickyIndex);

  const int headerIndex = displayModel_.fileHeaderRowIndex(config);
  if (headerIndex >= 0 && viewportY_ > 0.0) {
    appendRow(headerIndex);
  }

  for (int rowIndex = firstRow; rowIndex <= lastRow; ++rowIndex) {
    appendRow(rowIndex);
  }
  for (int margin = 1; margin <= kTilePrewarmRowMargin; ++margin) {
    appendRow(firstRow - margin);
    appendRow(lastRow + margin);
  }

  std::vector<TileSpec> specs;
  specs.reserve(rows.size() * 6);
  QSet<quint64> seenKeys;
  auto appendSpec = [&](TileSpec spec) {
    if (spec.targetRect.width() <= 0.0 || spec.targetRect.height() <= 0.0 || seenKeys.contains(spec.key)) {
      return;
    }
    seenKeys.insert(spec.key);
    specs.push_back(std::move(spec));
  };

  auto appendWholeRow = [&](TileLayer layer, int rowIndex, qreal logicalX, qreal widthForTile) {
    TileSpec spec;
    spec.layer = layer;
    spec.rowIndex = rowIndex;
    spec.columnIndex = 0;
    spec.logicalX = logicalX;
    spec.key = keyForTile(layer, rowIndex, 0);
    spec.targetRect = QRectF(0.0, 0.0, widthForTile, rows.at(rowIndex).height);
    appendSpec(std::move(spec));
  };

  auto appendTileColumns = [&](TileLayer layer, int rowIndex, qreal logicalOffset, qreal viewportSpan, qreal logicalWidth) {
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
      spec.key = keyForTile(layer, rowIndex, columnIndex);
      spec.targetRect = QRectF(0.0, 0.0, tileWidth, rows.at(rowIndex).height);
      appendSpec(std::move(spec));
    }
  };

  for (const int rowIndex : orderedRows) {
    const DiffDisplayRow& row = rows.at(rowIndex);
    if (mode == "split") {
      const bool useSplitPaneTiles = !wrapEnabled_;
      if (useSplitPaneTiles && row.rowType == DiffRowType::Line) {
        appendWholeRow(TileLayer::SplitLeftFixedRow, rowIndex, 0.0, leftPaneWidth);
        appendWholeRow(TileLayer::SplitRightFixedRow, rowIndex, 0.0, rightPaneWidth);
        appendTileColumns(TileLayer::SplitLeftTextRow, rowIndex, leftViewportX_, leftTextViewportWidth,
                          splitTextLogicalWidth);
        appendTileColumns(TileLayer::SplitRightTextRow, rowIndex, rightViewportX_, rightTextViewportWidth,
                          splitTextLogicalWidth);
      } else {
        appendWholeRow(TileLayer::SplitFullRow, rowIndex, 0.0, visibleWidth);
      }
    } else {
      appendTileColumns(TileLayer::UnifiedRow, rowIndex, viewportX_, visibleWidth, unifiedRowWidth);
    }
  }

  return specs;
}

void DiffSurfaceItem::scheduleAlternateTilePrewarm() {
  if (alternateTilePrewarmQueued_ || rowsModel_ == nullptr || rowsModel_->rows().empty() || width() <= 0.0 ||
      height() <= 0.0) {
    return;
  }

  alternateTilePrewarmQueued_ = true;
  QTimer::singleShot(0, this, [this]() {
    alternateTilePrewarmQueued_ = false;
    if (rowsModel_ == nullptr || rowsModel_->rows().empty() || width() <= 0.0 || height() <= 0.0) {
      return;
    }
    const QString alternateMode = layoutMode_ == "split" ? QStringLiteral("unified") : QStringLiteral("split");
    dispatchTileRaster(alternateMode, kAlternatePrewarmRasterPriority);
  });
}

void DiffSurfaceItem::invalidateRasterJobs(bool clearReadyImages) {
  rasterThreadPool_.clear();
  {
    const std::lock_guard<std::mutex> lock(readyTileImagesMutex_);
    if (clearReadyImages) {
      readyTileImages_.clear();
    }
  }
  {
    const std::lock_guard<std::mutex> lock(rasterJobStateMutex_);
    pendingRasterKeys_.clear();
    pendingTileJobCount_ = 0;
    ++rasterGeneration_;
  }
  updateTileStats();
}

std::shared_ptr<const DiffRasterSnapshot> DiffSurfaceItem::buildRasterSnapshot(const QString& mode,
                                                                              const QSet<int>& neededRows) {
  const DiffLayoutConfig config = buildLayoutConfig(mode);
  const auto& rows = displayModel_.cachedRows(config);
  if (rows.empty()) {
    return {};
  }

  auto snapshot = std::make_shared<DiffRasterSnapshot>();
  snapshot->generation = rasterGeneration_;
  snapshot->palette = palette_;
  snapshot->monoFontFamily = monoFontFamily_;
  snapshot->layoutMode = mode;
  snapshot->wrapEnabled = wrapEnabled_;
  snapshot->wrapColumn = wrapColumn_;
  snapshot->rowHeight = rowHeight_;
  snapshot->fileHeaderHeight = fileHeaderHeight_;
  snapshot->hunkHeight = hunkHeight_;
  snapshot->lineNumberDigits = lineNumberDigits_;
  snapshot->visibleWidth = width();
  snapshot->leftPaneWidth = width() / 2.0;
  snapshot->rightPaneWidth = width() - snapshot->leftPaneWidth;
  const qreal sideGutterWidth = 22.0 + digitWidth() * (lineNumberDigits_ + 1) + 12.0;
  const qreal splitTextInset = sideGutterWidth + 8.0;
  const qreal leftTextViewportWidth = std::max<qreal>(0.0, snapshot->leftPaneWidth - splitTextInset - 12.0);
  const qreal rightTextViewportWidth = std::max<qreal>(0.0, snapshot->rightPaneWidth - splitTextInset - 12.0);
  snapshot->unifiedRowWidth = std::max(width(), contentWidthForLayout(mode));
  snapshot->splitTextLogicalWidth =
      std::max({maxTextWidth_ + 12.0, leftTextViewportWidth, rightTextViewportWidth});
  snapshot->leftViewportX = leftViewportX_;
  snapshot->rightViewportX = rightViewportX_;
  snapshot->devicePixelRatio = window() != nullptr ? window()->effectiveDevicePixelRatio() : 1.0;
  snapshot->rows.reserve(neededRows.size());
  const auto& tokenBuf = displayModel_.tokenBuffer();
  for (const int rowIndex : neededRows) {
    if (rowIndex < 0 || rowIndex >= static_cast<int>(rows.size())) {
      continue;
    }
    const DiffDisplayRow& row = rows.at(rowIndex);
    DiffRasterRow rasterRow;
    rasterRow.row = row;
    if (!row.textRange.isEmpty()) {
      rasterRow.text = textForRange(row.textRange);
    }
    if (!row.leftTextRange.isEmpty()) {
      rasterRow.leftText = textForRange(row.leftTextRange);
    }
    if (!row.rightTextRange.isEmpty()) {
      rasterRow.rightText = textForRange(row.rightTextRange);
    }
    rasterRow.tokens.assign(tokenBuf.begin(row.tokens), tokenBuf.end(row.tokens));
    rasterRow.changeSpans.assign(tokenBuf.begin(row.changeSpans), tokenBuf.end(row.changeSpans));
    rasterRow.leftTokens.assign(tokenBuf.begin(row.leftTokens), tokenBuf.end(row.leftTokens));
    rasterRow.leftChangeSpans.assign(tokenBuf.begin(row.leftChangeSpans), tokenBuf.end(row.leftChangeSpans));
    rasterRow.rightTokens.assign(tokenBuf.begin(row.rightTokens), tokenBuf.end(row.rightTokens));
    rasterRow.rightChangeSpans.assign(tokenBuf.begin(row.rightChangeSpans), tokenBuf.end(row.rightChangeSpans));
    snapshot->rows.insert(rowIndex, std::move(rasterRow));
  }
  return snapshot;
}

void DiffSurfaceItem::queueRasterJobs(const std::shared_ptr<const DiffRasterSnapshot>& snapshot,
                                      const std::vector<TileSpec>& specs,
                                      int priority) {
  if (!snapshot || specs.empty()) {
    return;
  }

  for (const TileSpec& spec : specs) {
    {
      const std::lock_guard<std::mutex> lock(rasterJobStateMutex_);
      if (pendingRasterKeys_.contains(spec.key)) {
        continue;
      }
      pendingRasterKeys_.insert(spec.key);
      pendingTileJobCount_ = pendingRasterKeys_.size();
    }
    auto self = QPointer<DiffSurfaceItem>(this);
    auto onReady = [self, generation = snapshot->generation, key = spec.key](QImage image) mutable {
      if (!self) {
        return;
      }
      QMetaObject::invokeMethod(
          self,
          [self, generation, key, image = std::move(image)]() mutable {
            if (self) {
              self->acceptRasteredTile(generation, key, std::move(image));
            }
          },
          Qt::QueuedConnection);
    };
    auto* job = new RasterTileJob(snapshot, spec, std::move(onReady));
    job->setAutoDelete(true);
    rasterThreadPool_.start(job, priority);
  }
  updateTileStats();
}

void DiffSurfaceItem::dispatchTileRaster(const QString& mode, int priority) {
  const auto specs = buildPrewarmTileSpecs(mode);
  QSet<int> neededRows;
  for (const TileSpec& spec : specs) {
    neededRows.insert(spec.rowIndex);
  }
  const auto snapshot = buildRasterSnapshot(mode, neededRows);
  if (!snapshot) {
    return;
  }
  queueRasterJobs(snapshot, specs, priority);
}

void DiffSurfaceItem::acceptRasteredTile(quint64 generation, quint64 key, QImage image) {
  {
    const std::lock_guard<std::mutex> lock(rasterJobStateMutex_);
    if (generation != rasterGeneration_) {
      pendingRasterKeys_.remove(key);
      pendingTileJobCount_ = pendingRasterKeys_.size();
      updateTileStats();
      return;
    }
    pendingRasterKeys_.remove(key);
    pendingTileJobCount_ = pendingRasterKeys_.size();
  }
  {
    const std::lock_guard<std::mutex> lock(readyTileImagesMutex_);
    readyTileImages_.insert(key, std::move(image));
  }
  updateTileStats();
  update();
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
    connect(rowsModel_, &QAbstractItemModel::modelReset, this, [this]() { scheduleRowsRebuild(); });
    connect(rowsModel_, &QAbstractItemModel::rowsInserted, this, [this]() { scheduleRowsRebuild(); });
    connect(rowsModel_, &QAbstractItemModel::rowsRemoved, this, [this]() { scheduleRowsRebuild(); });
    connect(rowsModel_, &QAbstractItemModel::dataChanged, this, [this]() { scheduleRowsRebuild(); });
  }

  invalidateRasterJobs(true);
  scheduleRowsRebuild();
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
  viewportJumpFallbackArmed_ = false;
  invalidateRasterJobs();
  scheduleMetricsRecalc();
  emit layoutModeChanged();
}

int DiffSurfaceItem::compareGeneration() const {
  return compareGeneration_;
}

void DiffSurfaceItem::setCompareGeneration(int value) {
  if (compareGeneration_ == value) {
    return;
  }
  compareGeneration_ = value;
  invalidateRasterJobs(true);
  invalidateContentTiles();
  emit compareGenerationChanged();
}

QString DiffSurfaceItem::filePath() const {
  return filePath_;
}

void DiffSurfaceItem::setFilePath(const QString& path) {
  if (filePath_ == path) {
    return;
  }
  filePath_ = path;
  if (rowsRebuildQueued_) {
    updateFileHeader();
    emit filePathChanged();
    return;
  }
  if (updateFileHeader()) {
    invalidateRasterJobs();
    scheduleMetricsRecalc();
  } else {
    update();
    updateTileStats();
  }
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
  if (rowsRebuildQueued_) {
    updateFileHeader();
    emit fileStatusChanged();
    return;
  }
  if (updateFileHeader()) {
    invalidateRasterJobs();
    scheduleMetricsRecalc();
  } else {
    update();
    updateTileStats();
  }
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
  if (rowsRebuildQueued_) {
    updateFileHeader();
    emit additionsChanged();
    return;
  }
  if (updateFileHeader()) {
    invalidateRasterJobs();
    scheduleMetricsRecalc();
  } else {
    update();
    updateTileStats();
  }
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
  if (rowsRebuildQueued_) {
    updateFileHeader();
    emit deletionsChanged();
    return;
  }
  if (updateFileHeader()) {
    invalidateRasterJobs();
    scheduleMetricsRecalc();
  } else {
    update();
    updateTileStats();
  }
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
  invalidateRasterJobs();
  invalidatePaletteTiles();
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
  invalidateRasterJobs(true);
  scheduleRowsRebuild();
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
  viewportJumpFallbackArmed_ = false;
  invalidateRasterJobs();
  scheduleMetricsRecalc();
  emit wrapEnabledChanged();
}

int DiffSurfaceItem::wrapColumn() const {
  return wrapColumn_;
}

void DiffSurfaceItem::setWrapColumn(int value) {
  if (wrapColumn_ == value) return;
  wrapColumn_ = value;
  if (wrapEnabled_) {
    invalidateRasterJobs();
    scheduleMetricsRecalc();
  }
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
  scheduleCurrentTileRaster();
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
  scheduleCurrentTileRaster();
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
  scheduleCurrentTileRaster();
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
  const qreal delta = qAbs(viewportY_ - value);
  const bool largeJump = delta > std::max<qreal>(rowHeight_ * 2.0, viewportHeight_ * 0.15);
  viewportY_ = value;
  const int nextFirst = displayModel_.rowIndexAtY(std::max<qreal>(0.0, viewportY_ - hunkHeight_));
  const int nextLast = displayModel_.rowIndexAtY(viewportY_ + viewportHeight_ + hunkHeight_);
  const int nextSticky = displayModel_.stickyHunkRowIndexAtY(viewportY_);
  firstVisibleRow_ = nextFirst;
  lastVisibleRow_ = nextLast;
  stickyVisibleRow_ = nextSticky;
  if (largeJump) {
    viewportJumpFallbackArmed_ = true;
  }
  scheduleCurrentTileRaster();
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
  invalidateRasterJobs(false);
  scheduleCurrentTileRaster();
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

double DiffSurfaceItem::lastPaintTimeMs() const {
  return lastPaintTimeMs_;
}

double DiffSurfaceItem::lastRasterTimeMs() const {
  return lastRasterTimeMs_;
}

double DiffSurfaceItem::lastTextureUploadTimeMs() const {
  return lastTextureUploadTimeMs_;
}

double DiffSurfaceItem::lastRowsRebuildTimeMs() const {
  return lastRowsRebuildTimeMs_;
}

double DiffSurfaceItem::lastDisplayRowsRebuildTimeMs() const {
  return lastDisplayRowsRebuildTimeMs_;
}

double DiffSurfaceItem::lastMetricsRecalcTimeMs() const {
  return lastMetricsRecalcTimeMs_;
}

void DiffSurfaceItem::resetPerfStats() {
  paintCount_ = 0;
  tileCacheHits_ = 0;
  tileCacheMisses_ = 0;
  textureUploadCount_ = 0;
  lastPaintTimeMs_ = 0.0;
  lastRasterTimeMs_ = 0.0;
  lastTextureUploadTimeMs_ = 0.0;
  lastRowsRebuildTimeMs_ = 0.0;
  lastDisplayRowsRebuildTimeMs_ = 0.0;
  lastMetricsRecalcTimeMs_ = 0.0;
  emit paintCountChanged();
  emit tileStatsChanged();
  emit perfStatsChanged();
}

QSGNode* DiffSurfaceItem::updatePaintNode(QSGNode* oldNode, UpdatePaintNodeData* data) {
  Q_UNUSED(data);

  const PerfClock::time_point paintStart = PerfClock::now();
  auto* root = oldNode != nullptr ? oldNode : new QSGNode;
  auto clearChildren = [](QSGNode* parent) {
    while (QSGNode* child = parent->firstChild()) {
      parent->removeChildNode(child);
      delete child;
    }
  };

  auto* quickWindow = window();
  const auto& rows = displayModel_.rows();
  if (quickWindow == nullptr || width() <= 0 || height() <= 0) {
    clearChildren(root);
    return root;
  }

  if (rows.empty()) {
    clearChildren(root);
    return root;
  }

  const int uploadsBefore = textureUploadCount_;
  ++paintCount_;
  emit paintCountChanged();
  double rasterTimeMs = 0;
  double textureUploadTimeMs = 0;
  int syncRasterFallbackTiles = 0;

  const QRectF viewportClip = clipRect().intersected(QRectF(0.0, 0.0, width(), height()));
  const qreal visibleTopInItem = std::clamp(viewportClip.top(), 0.0, height());
  const qreal visibleBottomInItem = std::clamp(viewportClip.top() + viewportClip.height(), visibleTopInItem, height());
  const qreal visibleWidth = width();
  const qreal visibleHeight = visibleBottomInItem - visibleTopInItem;
  const qreal unifiedRowWidth = std::max(visibleWidth, contentWidth_);
  const qreal leftPaneWidth = visibleWidth / 2.0;
  const qreal rightPaneWidth = visibleWidth - leftPaneWidth;
  const qreal sideGutterWidth = 22.0 + digitWidth() * (lineNumberDigits_ + 1) + 12.0;
  const qreal splitTextInset = sideGutterWidth + 8.0;
  const qreal leftTextViewportWidth = std::max<qreal>(0.0, leftPaneWidth - splitTextInset - 12.0);
  const qreal rightTextViewportWidth = std::max<qreal>(0.0, rightPaneWidth - splitTextInset - 12.0);
  const qreal splitTextLogicalWidth = std::max({maxTextWidth_ + 12.0, leftTextViewportWidth, rightTextViewportWidth});
  const qreal devicePixelRatio = quickWindow->effectiveDevicePixelRatio();
  const quint64 currentTileContentKey = tileContentKey();
  const quint64 currentTileGeometryKey =
      tileGeometryKey(visibleWidth, visibleHeight, unifiedRowWidth, splitTextLogicalWidth);
  {
    const std::lock_guard<std::mutex> lock(readyTileImagesMutex_);
    if (!readyTileImages_.isEmpty()) {
      for (auto it = readyTileImages_.begin(); it != readyTileImages_.end(); ++it) {
        tileImageCache_.insert(it.key(), it.value());
      }
      readyTileImages_.clear();
    }
  }
  const int firstRow = std::max(0, displayModel_.rowIndexAtY(std::max<qreal>(0.0, viewportY_ + visibleTopInItem - hunkHeight_)));
  const int lastRow = std::max(firstRow, displayModel_.rowIndexAtY(viewportY_ + visibleBottomInItem + hunkHeight_));
  const int fileHeaderIndex = displayModel_.fileHeaderRowIndex();
  const bool hasStickyHeader = fileHeaderIndex >= 0 && viewportY_ > 0;
  const int stickyIndex = displayModel_.stickyHunkRowIndexAtY(viewportY_);
  double viewportRasterTimeMs = 0;
  int syncViewportFallbackTiles = 0;
  bool missingViewportCriticalTile = false;


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

  QSet<quint64> pinnedKeys;
  QSet<quint64> framePostedRasterKeys;
  std::vector<TileSpec> frameMissedSpecs;

  auto residentTextureForSpec = [&](const TileSpec& spec) -> QSGTexture* {
    ++tileUseTick_;
    pinnedKeys.insert(spec.key);
    const bool viewportCriticalSpec =
        spec.layer == TileLayer::StickyRow || (spec.rowIndex >= firstRow && spec.rowIndex <= lastRow);

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
      bool alreadyPending = false;
      {
        const std::lock_guard<std::mutex> lock(rasterJobStateMutex_);
        alreadyPending = pendingRasterKeys_.contains(spec.key);
      }
      const bool allowViewportFallback =
          viewportJumpFallbackArmed_ && viewportCriticalSpec &&
          syncViewportFallbackTiles < kViewportSyncRasterFallbackTileBudget &&
          viewportRasterTimeMs < kViewportSyncRasterFallbackMsBudget;
      const bool allowGeneralFallback = syncRasterFallbackTiles < kSyncRasterFallbackTileBudget &&
                                        rasterTimeMs < kSyncRasterFallbackMsBudget;
      if (allowViewportFallback || allowGeneralFallback) {
        const PerfClock::time_point rasterStart = PerfClock::now();
        image = renderTileImageInline(rows, spec, visibleWidth, unifiedRowWidth, splitTextLogicalWidth, leftPaneWidth,
                                      rightPaneWidth, devicePixelRatio);
        const double syncRasterMs = elapsedMs(rasterStart);
        rasterTimeMs += syncRasterMs;
        if (allowViewportFallback) {
          viewportRasterTimeMs += syncRasterMs;
          ++syncViewportFallbackTiles;
        } else {
          ++syncRasterFallbackTiles;
        }
        tileImageCache_.insert(spec.key, image);
        ++tileCacheMisses_;
      } else {
        if (viewportJumpFallbackArmed_ && viewportCriticalSpec) {
          missingViewportCriticalTile = true;
        }
        if (!alreadyPending && !framePostedRasterKeys.contains(spec.key)) {
          framePostedRasterKeys.insert(spec.key);
          ++tileCacheMisses_;
          frameMissedSpecs.push_back(spec);
        }
        return nullptr;
      }
    }
    tileImageLastUsed_[spec.key] = tileUseTick_;

    const PerfClock::time_point uploadStart = PerfClock::now();
    QSGTexture* texture = quickWindow->createTextureFromImage(image);
    textureUploadTimeMs += elapsedMs(uploadStart);
    Q_ASSERT(texture != nullptr);
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

  auto keyForTile = [&](TileLayer layer, int rowIndex, int columnIndex) {
    return tileKey(currentTileContentKey, currentTileGeometryKey, tilePaletteGeneration_, layer, rowIndex, columnIndex);
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
    spec.key = keyForTile(layer, rowIndex, 0);
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
      spec.key = keyForTile(layer, rowIndex, columnIndex);
      spec.targetRect = QRectF(baseX + logicalX - logicalOffset, y, tileWidth, rows.at(rowIndex).height);
      queueTextureNode(commands, spec);
    }
  };

  qreal stickyOffset = hasStickyHeader ? fileHeaderHeight_ : 0.0;
  qreal stickyViewportY = -1.0;
  if (stickyIndex >= 0) {
    qreal stickyY = viewportY_ + visibleTopInItem + stickyOffset;
    for (int nextIndex = stickyIndex + 1; nextIndex < static_cast<int>(rows.size()); ++nextIndex) {
      const DiffDisplayRow& nextRow = rows.at(nextIndex);
      if (nextRow.rowType == DiffRowType::Hunk) {
        stickyY = std::min(stickyY, nextRow.top - hunkHeight_);
        break;
      }
    }
    stickyViewportY = stickyY - viewportY_;
  }

  qreal occludedTop = 0.0;
  if (hasStickyHeader || visibleTopInItem > 0.0) {
    occludedTop = visibleTopInItem + (hasStickyHeader ? fileHeaderHeight_ : 0.0);
  }
  if (stickyViewportY >= 0.0) {
    occludedTop = std::max(occludedTop, stickyViewportY + hunkHeight_);
  }
  Q_ASSERT(occludedTop >= visibleTopInItem - 0.001);
  Q_ASSERT(occludedTop <= visibleBottomInItem + 0.001);

  baseCommands.reserve(static_cast<size_t>(std::max(16, lastRow - firstRow + 8)));
  leftTextCommands.reserve(static_cast<size_t>(std::max(8, lastRow - firstRow + 4)));
  rightTextCommands.reserve(static_cast<size_t>(std::max(8, lastRow - firstRow + 4)));
  overlayCommands.reserve(static_cast<size_t>(std::max(8, lastRow - firstRow + 4)));
  stickyCommands.reserve(4);
  QSGClipNode* rootClip = dynamic_cast<QSGClipNode*>(root->firstChild());
  if (rootClip == nullptr) {
    clearChildren(root);
    rootClip = createClipNode(QRectF(0.0, visibleTopInItem, visibleWidth, visibleHeight));
    root->appendChildNode(rootClip);
  } else {
    updateClipNodeRect(rootClip, QRectF(0.0, visibleTopInItem, visibleWidth, visibleHeight));
    while (QSGNode* extra = rootClip->nextSibling()) {
      root->removeChildNode(extra);
      delete extra;
    }
  }

  LayerClipNode* contentClip = dynamic_cast<LayerClipNode*>(rootClip->firstChild());
  LayerGroupNode* stickyGroup =
      contentClip != nullptr ? dynamic_cast<LayerGroupNode*>(contentClip->nextSibling()) : nullptr;
  LayerGroupNode* baseGroup = contentClip != nullptr ? dynamic_cast<LayerGroupNode*>(contentClip->firstChild()) : nullptr;
  TextClipNode* leftTextClip = baseGroup != nullptr ? dynamic_cast<TextClipNode*>(baseGroup->nextSibling()) : nullptr;
  TextClipNode* rightTextClip =
      leftTextClip != nullptr ? dynamic_cast<TextClipNode*>(leftTextClip->nextSibling()) : nullptr;
  LayerGroupNode* overlayGroup =
      rightTextClip != nullptr ? dynamic_cast<LayerGroupNode*>(rightTextClip->nextSibling()) : nullptr;

  const bool validLayerTree = contentClip != nullptr && contentClip->layer == 1 && stickyGroup != nullptr &&
                              stickyGroup->layer == 5 && stickyGroup->nextSibling() == nullptr &&
                              baseGroup != nullptr && baseGroup->layer == 2 && leftTextClip != nullptr &&
                              leftTextClip->layer == 2 && rightTextClip != nullptr && rightTextClip->layer == 3 &&
                              overlayGroup != nullptr && overlayGroup->layer == 4 && overlayGroup->nextSibling() == nullptr;

  if (!validLayerTree) {
    clearChildren(rootClip);

    contentClip = new LayerClipNode;
    contentClip->layer = 1;
    rootClip->appendChildNode(contentClip);

    baseGroup = new LayerGroupNode;
    baseGroup->layer = 2;
    contentClip->appendChildNode(baseGroup);

    leftTextClip = new TextClipNode;
    leftTextClip->layer = 2;
    contentClip->appendChildNode(leftTextClip);

    rightTextClip = new TextClipNode;
    rightTextClip->layer = 3;
    contentClip->appendChildNode(rightTextClip);

    overlayGroup = new LayerGroupNode;
    overlayGroup->layer = 4;
    contentClip->appendChildNode(overlayGroup);

    stickyGroup = new LayerGroupNode;
    stickyGroup->layer = 5;
    rootClip->appendChildNode(stickyGroup);
  }

  Q_ASSERT(contentClip != nullptr);
  Q_ASSERT(baseGroup != nullptr);
  Q_ASSERT(leftTextClip != nullptr);
  Q_ASSERT(rightTextClip != nullptr);
  Q_ASSERT(overlayGroup != nullptr);
  Q_ASSERT(stickyGroup != nullptr);
  Q_ASSERT(baseGroup->parent() == contentClip);
  Q_ASSERT(leftTextClip->parent() == contentClip);
  Q_ASSERT(rightTextClip->parent() == contentClip);
  Q_ASSERT(overlayGroup->parent() == contentClip);
  Q_ASSERT(stickyGroup->parent() == rootClip);

  updateClipNodeRect(contentClip, QRectF(0.0, occludedTop, visibleWidth,
                                         std::max<qreal>(0.0, visibleBottomInItem - occludedTop)));
  updateClipNodeRect(leftTextClip,
                     QRectF(splitTextInset, occludedTop, leftTextViewportWidth,
                            std::max<qreal>(0.0, visibleBottomInItem - occludedTop)));
  updateClipNodeRect(rightTextClip,
                     QRectF(leftPaneWidth + splitTextInset, occludedTop, rightTextViewportWidth,
                            std::max<qreal>(0.0, visibleBottomInItem - occludedTop)));


  queueRectNode(baseCommands, overlayKey(0, -1, 0), QRectF(0.0, 0.0, visibleWidth, visibleHeight),
                paletteColor("canvas", QColor("#282c33")));

  if (layoutMode_ == "split") {
    const bool useSplitPaneTiles = !wrapEnabled_;
    for (int rowIndex = firstRow; rowIndex <= lastRow && rowIndex < static_cast<int>(rows.size()); ++rowIndex) {
      const DiffDisplayRow& row = rows.at(rowIndex);
      const qreal y = row.top - viewportY_;
      if (y + row.height < visibleTopInItem || y > visibleBottomInItem) {
        continue;
      }

      if (useSplitPaneTiles && row.rowType == DiffRowType::Line) {
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
      if (y + row.height < visibleTopInItem || y > visibleBottomInItem) {
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

  if (stickyIndex >= 0) {
    queueRectNode(stickyCommands, overlayKey(6, stickyIndex, 0), QRectF(0.0, stickyViewportY, visibleWidth, hunkHeight_),
                  paletteColor("canvas", QColor("#282828")));
    const TileSpec stickyHunkSpec{keyForTile(TileLayer::StickyRow, stickyIndex, 0), TileLayer::StickyRow, stickyIndex,
                                  0, 0.0, QRectF(0.0, stickyViewportY, visibleWidth, rows.at(stickyIndex).height)};
    queueTextureNode(stickyCommands, stickyHunkSpec);
  }

  if (hasStickyHeader) {
    queueRectNode(stickyCommands, overlayKey(5, fileHeaderIndex, 0),
                  QRectF(0.0, visibleTopInItem, visibleWidth, fileHeaderHeight_),
                  paletteColor("canvas", QColor("#282828")));
    const TileSpec stickyHeaderSpec{keyForTile(TileLayer::StickyRow, fileHeaderIndex, 0), TileLayer::StickyRow,
                                    fileHeaderIndex, 0, 0.0,
                                    QRectF(0.0, visibleTopInItem, visibleWidth, rows.at(fileHeaderIndex).height)};
    queueTextureNode(stickyCommands, stickyHeaderSpec);
  }

  if (layoutMode_ != "split") {
    Q_ASSERT(leftTextCommands.empty());
    Q_ASSERT(rightTextCommands.empty());
  }

  auto reconcileCommands = [&](QSGNode* parent, const std::vector<RenderCommand>& commands) {
    QHash<quint64, QSGNode*> detachedByKey;
    QVector<QSGNode*> anonymousNodes;
    while (QSGNode* child = parent->firstChild()) {
      parent->removeChildNode(child);
      if (auto* textureNode = dynamic_cast<TextureTileNode*>(child)) {
        detachedByKey.insert(textureNode->key, child);
      } else if (auto* rectNode = dynamic_cast<RectTileNode*>(child)) {
        detachedByKey.insert(rectNode->key, child);
      } else {
        anonymousNodes.push_back(child);
      }
    }
    qDeleteAll(anonymousNodes);

    for (const RenderCommand& command : commands) {
      if (command.kind == RenderCommand::Kind::Texture) {
        auto* node = dynamic_cast<TextureTileNode*>(detachedByKey.take(command.key));
        if (node == nullptr) {
          node = new TextureTileNode;
          node->setFiltering(QSGTexture::Nearest);
          node->setOwnsTexture(false);
        }
        node->key = command.key;
        QSGTexture* texture = residentTextureForSpec(command.spec);
        if (texture == nullptr) {
          delete node;
          continue;
        }
        node->setTexture(texture);
        node->setRect(command.spec.targetRect);
        parent->appendChildNode(node);
        continue;
      }

      auto* node = dynamic_cast<RectTileNode*>(detachedByKey.take(command.key));
      if (node == nullptr) {
        node = new RectTileNode;
      }
      node->key = command.key;
      node->setRect(command.rect);
      node->setColor(command.color);
      parent->appendChildNode(node);
    }

    for (auto it = detachedByKey.begin(); it != detachedByKey.end(); ++it) {
      delete it.value();
    }
  };

  reconcileCommands(stickyGroup, stickyCommands);
  reconcileCommands(baseGroup, baseCommands);
  reconcileCommands(leftTextClip, leftTextCommands);
  reconcileCommands(rightTextClip, rightTextCommands);
  reconcileCommands(overlayGroup, overlayCommands);

  auto evictCache = [&](int limit, auto& cache, auto& lastUsed, auto deleter) {
    if (cache.size() <= limit) {
      return;
    }
    const int excess = cache.size() - limit;
    std::vector<std::pair<quint64, quint64>> candidates;
    candidates.reserve(cache.size());
    for (auto it = cache.begin(); it != cache.end(); ++it) {
      if (!pinnedKeys.contains(it.key())) {
        candidates.emplace_back(lastUsed.value(it.key(), 0), it.key());
      }
    }
    const int evictCount = std::min(excess, static_cast<int>(candidates.size()));
    if (evictCount <= 0) {
      return;
    }
    std::partial_sort(candidates.begin(), candidates.begin() + evictCount, candidates.end());
    for (int i = 0; i < evictCount; ++i) {
      deleter(candidates[i].second);
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

  if (!frameMissedSpecs.empty()) {
    const QString batchMode = layoutMode_;
    QSet<int> neededRows;
    for (const TileSpec& spec : frameMissedSpecs) {
      neededRows.insert(spec.rowIndex);
    }
    QMetaObject::invokeMethod(
        this,
        [this, specs = std::move(frameMissedSpecs), batchMode, neededRows = std::move(neededRows)]() {
          const auto snapshot = buildRasterSnapshot(batchMode, neededRows);
          if (!snapshot) {
            return;
          }
          queueRasterJobs(snapshot, specs, kVisibleTileRequestPriority);
        },
        Qt::QueuedConnection);
  }

  if (viewportJumpFallbackArmed_ && !missingViewportCriticalTile) {
    viewportJumpFallbackArmed_ = false;
  }

  if ((missingViewportCriticalTile || textureUploadCount_ > uploadsBefore) && !followupUpdateQueued_) {
    followupUpdateQueued_ = true;
    QTimer::singleShot(1, this, [this]() {
      followupUpdateQueued_ = false;
      update();
    });
  }

  updateTileStats();
  bool perfChanged = false;
  perfChanged |= setPerfValue(lastPaintTimeMs_, elapsedMs(paintStart));
  perfChanged |= setPerfValue(lastRasterTimeMs_, rasterTimeMs);
  perfChanged |= setPerfValue(lastTextureUploadTimeMs_, textureUploadTimeMs);
  if (perfChanged) {
    emit perfStatsChanged();
  }

  return root;
}

void DiffSurfaceItem::releaseResources() {
  rasterThreadPool_.clear();
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
  const PerfClock::time_point rebuildStart = PerfClock::now();
  textCache_.clear();
  leftViewportX_ = 0;
  rightViewportX_ = 0;

  const QFontMetricsF metrics(monoFont(monoFontFamily_, 12));
  const TextWidthMeasure measureTextWidth = [&metrics](std::string_view text) {
    return metrics.horizontalAdvance(QString::fromUtf8(text.data(), static_cast<qsizetype>(text.size())));
  };
  lineHeight_ = metrics.height();
  rowHeight_ = qCeil(lineHeight_ + 8.0);
  fileHeaderHeight_ = 32.0;
  hunkHeight_ = 28.0;
  const PreparedRowsCacheKey key = preparedRowsCacheKey();
  const PreparedRows* prepared = rowsModel_ != nullptr ? rowsModel_->preparedRows(key) : nullptr;
  PreparedRows localPrepared;
  if (prepared != nullptr) {
    textBuffer_ = prepared->textBuffer;
    maxTextWidth_ = prepared->maxTextWidth;
    displayModel_.setSourceRows(prepared->sourceRows, prepared->tokenBuffer);
  } else {
    if (rowsModel_ != nullptr) {
      rowsModel_->storePreparedRows(key, prepareRowsForDisplay(rowsModel_->rows(), measureTextWidth));
      prepared = rowsModel_->preparedRows(key);
    }
    if (prepared == nullptr) {
      localPrepared = prepareRowsForDisplay({}, measureTextWidth);
      prepared = &localPrepared;
    }
    textBuffer_ = prepared->textBuffer;
    maxTextWidth_ = prepared->maxTextWidth;
    displayModel_.setSourceRows(prepared->sourceRows, prepared->tokenBuffer);
  }
  updateFileHeader();
  if (setPerfValue(lastRowsRebuildTimeMs_, elapsedMs(rebuildStart))) {
    emit perfStatsChanged();
  }
  recalculateMetrics();
}

void DiffSurfaceItem::rebuildDisplayRows() {
  const PerfClock::time_point rebuildStart = PerfClock::now();
  const DiffLayoutConfig config = buildLayoutConfig(layoutMode_);
  displayModel_.rebuild(config);
  contentHeight_ = displayModel_.contentHeight();
  lineNumberDigits_ = displayModel_.lineNumberDigits();
  firstVisibleRow_ = displayModel_.rowIndexAtY(std::max<qreal>(0.0, viewportY_ - hunkHeight_));
  lastVisibleRow_ = displayModel_.rowIndexAtY(viewportY_ + viewportHeight_ + hunkHeight_);
  stickyVisibleRow_ = displayModel_.stickyHunkRowIndexAtY(viewportY_);
  emit displayRowCountChanged();
  if (setPerfValue(lastDisplayRowsRebuildTimeMs_, elapsedMs(rebuildStart))) {
    emit perfStatsChanged();
  }
}

void DiffSurfaceItem::recalculateMetrics() {
  const PerfClock::time_point recalcStart = PerfClock::now();
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
  scheduleCurrentTileRaster();
  update();
  updateTileStats();
  if (setPerfValue(lastMetricsRecalcTimeMs_, elapsedMs(recalcStart))) {
    emit perfStatsChanged();
  }
  scheduleAlternateLayoutPrewarm();
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
  const std::string_view sv = textBuffer_.view(range);
  const QString text = QString::fromUtf8(sv.data(), static_cast<qsizetype>(sv.size()));
  textCache_.insert(key, text);
  return text;
}

const DiffSurfaceItem::CachedLineLayout& DiffSurfaceItem::lineLayoutForText(const QString& text, int pixelSize) const {
  LineLayoutCacheKey key;
  key.text = text;
  key.family = monoFontFamily_;
  key.pixelSize = pixelSize;
  ++lineLayoutUseTick_;
  if (const auto it = lineLayoutCache_.constFind(key); it != lineLayoutCache_.constEnd()) {
    lineLayoutLastUsed_.insert(key, lineLayoutUseTick_);
    return it.value();
  }

  CachedLineLayout layout;
  const QFontMetricsF metrics(monoFont(monoFontFamily_, pixelSize));
  const qreal charWidth = metrics.horizontalAdvance(QLatin1Char('M'));
  layout.prefixAdvances.reserve(static_cast<size_t>(text.size() + 1));
  layout.prefixAdvances.push_back(0.0);
  for (int i = 0; i < text.size(); ++i) {
    layout.prefixAdvances.push_back(charWidth * (i + 1));
  }
  layout.width = text.isEmpty() ? 0.0 : charWidth * text.size();

  auto inserted = lineLayoutCache_.insert(key, std::move(layout));
  lineLayoutLastUsed_.insert(key, lineLayoutUseTick_);
  while (lineLayoutCache_.size() > kMaxLineLayoutCacheEntries) {
    auto victimIt = lineLayoutLastUsed_.cbegin();
    for (auto it = lineLayoutLastUsed_.cbegin(); it != lineLayoutLastUsed_.cend(); ++it) {
      if (it.value() < victimIt.value()) {
        victimIt = it;
      }
    }
    if (victimIt == lineLayoutLastUsed_.cend()) {
      break;
    }
    for (auto it = wrappedLayoutCache_.begin(); it != wrappedLayoutCache_.end();) {
      if (it.key().base == victimIt.key()) {
        it = wrappedLayoutCache_.erase(it);
      } else {
        ++it;
      }
    }
    for (auto it = wrappedLayoutLastUsed_.begin(); it != wrappedLayoutLastUsed_.end();) {
      if (it.key().base == victimIt.key()) {
        it = wrappedLayoutLastUsed_.erase(it);
      } else {
        ++it;
      }
    }
    lineLayoutCache_.remove(victimIt.key());
    lineLayoutLastUsed_.erase(victimIt);
  }
  return inserted.value();
}

const DiffSurfaceItem::CachedLineLayout& DiffSurfaceItem::lineLayoutForRange(const TextRange& range, int pixelSize) const {
  return lineLayoutForText(textForRange(range), pixelSize);
}

const DiffSurfaceItem::CachedWrappedLayout& DiffSurfaceItem::wrappedLayoutForText(const QString& text,
                                                                                  int pixelSize,
                                                                                  qreal wrapWidth) const {
  WrappedLineLayoutCacheKey key;
  key.base.text = text;
  key.base.family = monoFontFamily_;
  key.base.pixelSize = pixelSize;
  key.wrapWidthMilli = wrapWidthCacheKey(wrapWidth);
  ++wrappedLayoutUseTick_;
  if (const auto it = wrappedLayoutCache_.constFind(key); it != wrappedLayoutCache_.constEnd()) {
    wrappedLayoutLastUsed_.insert(key, wrappedLayoutUseTick_);
    return it.value();
  }

  CachedWrappedLayout wrappedLayout;
  const auto& layout = lineLayoutForText(text, pixelSize);
  wrappedLayout.charWrapLines.resize(layout.prefixAdvances.size(), 0);
  if (wrapWidth > 0.0 && !layout.prefixAdvances.empty()) {
    int currentLine = 0;
    qreal nextBoundary = wrapWidth;
    for (size_t index = 0; index < layout.prefixAdvances.size(); ++index) {
      while (layout.prefixAdvances[index] >= nextBoundary) {
        ++currentLine;
        nextBoundary += wrapWidth;
      }
      wrappedLayout.charWrapLines[index] = currentLine;
    }
    wrappedLayout.lineCount = currentLine + 1;
  }

  auto inserted = wrappedLayoutCache_.insert(key, std::move(wrappedLayout));
  wrappedLayoutLastUsed_.insert(key, wrappedLayoutUseTick_);
  while (wrappedLayoutCache_.size() > kMaxWrappedLayoutCacheEntries) {
    auto victimIt = wrappedLayoutLastUsed_.cbegin();
    for (auto it = wrappedLayoutLastUsed_.cbegin(); it != wrappedLayoutLastUsed_.cend(); ++it) {
      if (it.value() < victimIt.value()) {
        victimIt = it;
      }
    }
    if (victimIt == wrappedLayoutLastUsed_.cend()) {
      break;
    }
    wrappedLayoutCache_.remove(victimIt.key());
    wrappedLayoutLastUsed_.erase(victimIt);
  }
  return inserted.value();
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

PreparedRowsCacheKey DiffSurfaceItem::preparedRowsCacheKey() const {
  PreparedRowsCacheKey key;
  key.compareGeneration = compareGeneration_;
  key.filePath = filePath_.toStdString();
  key.family = monoFontFamily_.toStdString();
  return key;
}

quint64 DiffSurfaceItem::tileContentKey() const {
  return qHashMulti(0u, compareGeneration_, filePath_, fileStatus_, additions_, deletions_);
}

quint64 DiffSurfaceItem::tileGeometryKey(const QString& mode,
                                        qreal contentWidth,
                                        qreal visibleWidth,
                                        qreal visibleHeight,
                                        qreal unifiedRowWidth,
                                        qreal splitTextLogicalWidth) const {
  return qHashMulti(0u, mode, wrapEnabled_, wrapColumn_, qRound64(width() * 100.0), qRound64(height() * 100.0),
                    qRound64(visibleWidth * 100.0), qRound64(visibleHeight * 100.0),
                    qRound64(contentWidth * 100.0), qRound64(unifiedRowWidth * 100.0),
                    qRound64(splitTextLogicalWidth * 100.0), qRound64(maxTextWidth_ * 100.0), lineNumberDigits_,
                    qRound64(rowHeight_ * 100.0), qRound64(fileHeaderHeight_ * 100.0), qRound64(hunkHeight_ * 100.0));
}

quint64 DiffSurfaceItem::tileGeometryKey(qreal visibleWidth,
                                        qreal visibleHeight,
                                        qreal unifiedRowWidth,
                                        qreal splitTextLogicalWidth) const {
  return tileGeometryKey(layoutMode_, contentWidth_, visibleWidth, visibleHeight, unifiedRowWidth, splitTextLogicalWidth);
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
  const QString text = textForRange(row.textRange);
  const CachedLineLayout& layout = lineLayoutForRange(row.textRange, 12);
  const auto& tokenBuf = displayModel_.tokenBuffer();
  drawTextRun(painter, QPointF(textClip.left(), unifiedBaselineY), textClip, text,
              tokenBuf.begin(row.tokens), row.tokens.count,
              tokenBuf.begin(row.changeSpans), row.changeSpans.count,
              layout.prefixAdvances, paletteColor("textBase", QColor("#c8ccd4")), tokenBg);
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

  QColor background = splitPaneBackgroundColor(row, isLeftPane);
  if (spacer) {
    background = paletteColor("lineContextAlt", background);
  } else if (isLeftPane && lineKind == DiffLineKind::Deletion) {
    background = paletteColor("lineDelAccent", background);
  } else if (!isLeftPane && lineKind == DiffLineKind::Addition) {
    background = paletteColor("lineAddAccent", background);
  } else {
    background = paletteColor("lineContext", background);
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
  const DiffLineKind lineKind = isLeftPane ? row.leftKind : row.rightKind;
  QColor background = splitPaneBackgroundColor(row, isLeftPane);
  if (lineKind == DiffLineKind::Spacer) {
    background = paletteColor("lineContextAlt", background);
  } else if (isLeftPane && lineKind == DiffLineKind::Deletion) {
    background = paletteColor("lineDelAccent", background);
  } else if (!isLeftPane && lineKind == DiffLineKind::Addition) {
    background = paletteColor("lineAddAccent", background);
  } else {
    background = paletteColor("lineContext", background);
  }
  painter->fillRect(rowRect, background);

  const bool spacer = isLeftPane ? row.leftKind == DiffLineKind::Spacer : row.rightKind == DiffLineKind::Spacer;
  if (spacer) {
    return;
  }

  const TextRange& textRange = isLeftPane ? row.leftTextRange : row.rightTextRange;
  const TokenRange& tokens = isLeftPane ? row.leftTokens : row.rightTokens;
  const TokenRange& changeSpans = isLeftPane ? row.leftChangeSpans : row.rightChangeSpans;
  const QString text = textForRange(textRange);
  const CachedLineLayout& layout = lineLayoutForRange(textRange, 12);
  const QFont textFont = monoFont(monoFontFamily_, 12);
  const QFontMetricsF textMetrics(textFont);
  painter->setFont(textFont);
  const qreal baselineY = wrapEnabled_
      ? rowRect.top() + (rowHeight_ - textMetrics.height()) / 2.0 + textMetrics.ascent()
      : rowRect.top() + (rowRect.height() - textMetrics.height()) / 2.0 + textMetrics.ascent();
  const auto& tokenBuf = displayModel_.tokenBuffer();
  drawTextRun(painter, QPointF(rowRect.left(), baselineY), rowRect, text,
              tokenBuf.begin(tokens), tokens.count,
              tokenBuf.begin(changeSpans), changeSpans.count,
              layout.prefixAdvances, paletteColor("textBase", QColor("#c8ccd4")),
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
                                  const DiffTokenSpan* tokens,
                                  size_t tokenCount,
                                  const DiffTokenSpan* changeSpans,
                                  size_t changeSpanCount,
                                  const std::vector<qreal>& charX,
                                  const QColor& textColor,
                                  const QColor& tokenBackground) const {
  painter->save();
  painter->setClipRect(clipRect);

  const QFont textFont = monoFont(monoFontFamily_, 12);
  const QFontMetricsF metrics(textFont);
  painter->setFont(textFont);

  if (wrapEnabled_) {
    drawTextRunWrapped(painter, baseline, clipRect, text, tokens, tokenCount, changeSpans, changeSpanCount, charX,
                       textColor, tokenBackground, metrics);
    painter->restore();
    return;
  }

  for (size_t i = 0; i < changeSpanCount; ++i) {
    const DiffTokenSpan& span = changeSpans[i];
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
  std::vector<DiffTokenSpan> sortedTokens(tokens, tokens + tokenCount);
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
                                          const DiffTokenSpan* tokens,
                                          size_t tokenCount,
                                          const DiffTokenSpan* changeSpans,
                                          size_t changeSpanCount,
                                          const std::vector<qreal>& charX,
                                          const QColor& textColor,
                                          const QColor& tokenBackground,
                                          const QFontMetricsF& metrics) const {
  const auto& wrappedLayout = wrappedLayoutForText(text, 12, clipRect.width());
  const qreal lineH = metrics.height() + 2.0;
  const qreal originX = clipRect.left();

  auto wrapLine = [&](int charIdx) -> int {
    if (wrappedLayout.charWrapLines.empty()) {
      return 0;
    }
    const int clampedIndex = std::clamp(charIdx, 0, static_cast<int>(wrappedLayout.charWrapLines.size()) - 1);
    return wrappedLayout.charWrapLines[clampedIndex];
  };

  auto xForChar = [&](int charIdx) -> qreal {
    const qreal availWidth = clipRect.width();
    return originX + charX[charIdx] - wrapLine(charIdx) * availWidth;
  };

  auto yForChar = [&](int charIdx) -> qreal {
    return baseline.y() + wrapLine(charIdx) * lineH;
  };

  for (size_t i = 0; i < changeSpanCount; ++i) {
    const DiffTokenSpan& span = changeSpans[i];
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
  std::vector<DiffTokenSpan> sortedTokens(tokens, tokens + tokenCount);
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
  const int newHoveredRow = displayModel_.rowIndexAtY(event->position().y() + viewportY_);
  if (newHoveredRow != hoveredRow_) {
    hoveredRow_ = newHoveredRow;
    update();
  }
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
