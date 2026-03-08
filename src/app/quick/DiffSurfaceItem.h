#pragma once

#include <QFontMetricsF>
#include <QHash>
#include <QHoverEvent>
#include <QImage>
#include <QKeyEvent>
#include <QMouseEvent>
#include <QQuickItem>
#include <QRectF>
#include <QSet>
#include <QThreadPool>
#include <QWheelEvent>
#include <QVariantMap>

#include <memory>
#include <mutex>
#include <vector>

#include "app/models/DiffRowListModel.h"
#include "core/rendering/DiffLayoutEngine.h"
#include "core/rendering/PreparedRows.h"
#include "core/text/TextRope.h"

class QPainter;
class QSGNode;
class QSGTexture;

namespace diffy {

struct DiffRasterSnapshot;

struct LineLayoutCacheKey {
  QString text;
  QString family;
  int pixelSize = 0;

  bool operator==(const LineLayoutCacheKey& other) const = default;
};

struct WrappedLineLayoutCacheKey {
  LineLayoutCacheKey base;
  qint64 wrapWidthMilli = -1;

  bool operator==(const WrappedLineLayoutCacheKey& other) const = default;
};

inline size_t qHash(const LineLayoutCacheKey& key, size_t seed = 0) {
  return qHashMulti(seed, key.text, key.family, key.pixelSize);
}

inline size_t qHash(const WrappedLineLayoutCacheKey& key, size_t seed = 0) {
  return qHashMulti(seed, key.base, key.wrapWidthMilli);
}

enum class TileLayer {
  UnifiedRow = 1,
  SplitFullRow = 2,
  SplitLeftFixedRow = 3,
  SplitRightFixedRow = 4,
  SplitLeftTextRow = 5,
  SplitRightTextRow = 6,
  StickyRow = 7,
};

struct TileSpec {
  quint64 key = 0;
  TileLayer layer = TileLayer::UnifiedRow;
  int rowIndex = -1;
  int columnIndex = 0;
  qreal logicalX = 0.0;
  QRectF targetRect;
};

class DiffSurfaceItem : public QQuickItem {
  Q_OBJECT
  Q_PROPERTY(QObject* rowsModel READ rowsModel WRITE setRowsModel NOTIFY rowsModelChanged)
  Q_PROPERTY(QString layoutMode READ layoutMode WRITE setLayoutMode NOTIFY layoutModeChanged)
  Q_PROPERTY(int compareGeneration READ compareGeneration WRITE setCompareGeneration NOTIFY compareGenerationChanged)
  Q_PROPERTY(QString filePath READ filePath WRITE setFilePath NOTIFY filePathChanged)
  Q_PROPERTY(QString fileStatus READ fileStatus WRITE setFileStatus NOTIFY fileStatusChanged)
  Q_PROPERTY(int additions READ additions WRITE setAdditions NOTIFY additionsChanged)
  Q_PROPERTY(int deletions READ deletions WRITE setDeletions NOTIFY deletionsChanged)
  Q_PROPERTY(QVariantMap palette READ palette WRITE setPalette NOTIFY paletteChanged)
  Q_PROPERTY(QString monoFontFamily READ monoFontFamily WRITE setMonoFontFamily NOTIFY monoFontFamilyChanged)
  Q_PROPERTY(qreal contentHeight READ contentHeight NOTIFY contentHeightChanged)
  Q_PROPERTY(qreal contentWidth READ contentWidth NOTIFY contentWidthChanged)
  Q_PROPERTY(qreal viewportX READ viewportX WRITE setViewportX NOTIFY viewportXChanged)
  Q_PROPERTY(qreal viewportY READ viewportY WRITE setViewportY NOTIFY viewportYChanged)
  Q_PROPERTY(qreal leftViewportX READ leftViewportX WRITE setLeftViewportX NOTIFY leftViewportXChanged)
  Q_PROPERTY(qreal rightViewportX READ rightViewportX WRITE setRightViewportX NOTIFY rightViewportXChanged)
  Q_PROPERTY(qreal viewportHeight READ viewportHeight WRITE setViewportHeight NOTIFY viewportHeightChanged)
  Q_PROPERTY(bool wrapEnabled READ wrapEnabled WRITE setWrapEnabled NOTIFY wrapEnabledChanged)
  Q_PROPERTY(int wrapColumn READ wrapColumn WRITE setWrapColumn NOTIFY wrapColumnChanged)
  Q_PROPERTY(int paintCount READ paintCount NOTIFY paintCountChanged)
  Q_PROPERTY(int displayRowCount READ displayRowCount NOTIFY displayRowCountChanged)
  Q_PROPERTY(int tileCacheHits READ tileCacheHits NOTIFY tileStatsChanged)
  Q_PROPERTY(int tileCacheMisses READ tileCacheMisses NOTIFY tileStatsChanged)
  Q_PROPERTY(int textureUploadCount READ textureUploadCount NOTIFY tileStatsChanged)
  Q_PROPERTY(int residentTileCount READ residentTileCount NOTIFY tileStatsChanged)
  Q_PROPERTY(int pendingTileJobCount READ pendingTileJobCount NOTIFY tileStatsChanged)
  Q_PROPERTY(double lastPaintTimeMs READ lastPaintTimeMs NOTIFY perfStatsChanged)
  Q_PROPERTY(double lastRasterTimeMs READ lastRasterTimeMs NOTIFY perfStatsChanged)
  Q_PROPERTY(double lastTextureUploadTimeMs READ lastTextureUploadTimeMs NOTIFY perfStatsChanged)
  Q_PROPERTY(double lastRowsRebuildTimeMs READ lastRowsRebuildTimeMs NOTIFY perfStatsChanged)
  Q_PROPERTY(double lastDisplayRowsRebuildTimeMs READ lastDisplayRowsRebuildTimeMs NOTIFY perfStatsChanged)
  Q_PROPERTY(double lastMetricsRecalcTimeMs READ lastMetricsRecalcTimeMs NOTIFY perfStatsChanged)

 public:
  explicit DiffSurfaceItem(QQuickItem* parent = nullptr);

  QObject* rowsModel() const;
  void setRowsModel(QObject* model);

  QString layoutMode() const;
  void setLayoutMode(const QString& mode);
  int compareGeneration() const;
  void setCompareGeneration(int value);

  QString filePath() const;
  void setFilePath(const QString& path);

  QString fileStatus() const;
  void setFileStatus(const QString& status);

  int additions() const;
  void setAdditions(int value);

  int deletions() const;
  void setDeletions(int value);

  QVariantMap palette() const;
  void setPalette(const QVariantMap& palette);

  QString monoFontFamily() const;
  void setMonoFontFamily(const QString& family);

  qreal contentHeight() const;
  qreal contentWidth() const;
  qreal viewportX() const;
  void setViewportX(qreal value);

  qreal viewportY() const;
  void setViewportY(qreal value);

  qreal leftViewportX() const;
  void setLeftViewportX(qreal value);

  qreal rightViewportX() const;
  void setRightViewportX(qreal value);

  qreal viewportHeight() const;
  void setViewportHeight(qreal value);

  bool wrapEnabled() const;
  void setWrapEnabled(bool value);
  int wrapColumn() const;
  void setWrapColumn(int value);

  int paintCount() const;
  int displayRowCount() const;
  int tileCacheHits() const;
  int tileCacheMisses() const;
  int textureUploadCount() const;
  int residentTileCount() const;
  int pendingTileJobCount() const;
  double lastPaintTimeMs() const;
  double lastRasterTimeMs() const;
  double lastTextureUploadTimeMs() const;
  double lastRowsRebuildTimeMs() const;
  double lastDisplayRowsRebuildTimeMs() const;
  double lastMetricsRecalcTimeMs() const;
  Q_INVOKABLE void resetPerfStats();

signals:
  void rowsModelChanged();
  void layoutModeChanged();
  void compareGenerationChanged();
  void filePathChanged();
  void fileStatusChanged();
  void additionsChanged();
  void deletionsChanged();
  void paletteChanged();
  void monoFontFamilyChanged();
  void contentHeightChanged();
  void contentWidthChanged();
  void viewportXChanged();
  void viewportYChanged();
  void leftViewportXChanged();
  void rightViewportXChanged();
  void viewportHeightChanged();
  void wrapEnabledChanged();
  void wrapColumnChanged();
  void paintCountChanged();
  void displayRowCountChanged();
  void tileStatsChanged();
  void perfStatsChanged();
  void scrollToYRequested(qreal value);
  void nextFileRequested();
  void previousFileRequested();

 private:
  void scheduleRowsRebuild();
  void scheduleMetricsRecalc();
  void scheduleAlternateLayoutPrewarm();
  void scheduleCurrentTileRaster();
  DiffLayoutConfig buildLayoutConfig(const QString& mode) const;
  void scheduleAlternateTilePrewarm();
  void dispatchTileRaster(const QString& mode, int priority);
  void invalidateRasterJobs(bool clearReadyImages = false);
  bool updateFileHeader();
  void rebuildRows();
  void rebuildDisplayRows();
  void recalculateMetrics();
  void invalidateContentTiles();
  void invalidateGeometryTiles();
  void invalidatePaletteTiles();
  void updateTileStats();

  struct CachedLineLayout {
    qreal width = 0;
    std::vector<qreal> prefixAdvances;
  };

  struct CachedWrappedLayout {
    int lineCount = 1;
    std::vector<int> charWrapLines;
  };

  bool rowSelected(int rowIndex) const;
  QColor paletteColor(const QString& key, const QColor& fallback) const;
  qreal digitWidth() const;
  qreal unifiedGutterWidth() const;
  QString selectedText() const;
  QString textForRange(const TextRange& range) const;
  const CachedLineLayout& lineLayoutForText(const QString& text, int pixelSize) const;
  const CachedLineLayout& lineLayoutForRange(const TextRange& range, int pixelSize) const;
  const CachedWrappedLayout& wrappedLayoutForText(const QString& text, int pixelSize, qreal wrapWidth) const;
  int currentRowIndex() const;
  PreparedRowsCacheKey preparedRowsCacheKey() const;
  std::shared_ptr<const DiffRasterSnapshot> buildRasterSnapshot(const QString& mode);
  qreal contentWidthForLayout(const QString& mode) const;
  std::vector<TileSpec> buildPrewarmTileSpecs(const QString& mode);
  QImage renderTileImageInline(const std::vector<DiffDisplayRow>& rows,
                               const TileSpec& spec,
                               qreal visibleWidth,
                               qreal unifiedRowWidth,
                               qreal splitTextLogicalWidth,
                               qreal leftPaneWidth,
                               qreal rightPaneWidth,
                               qreal devicePixelRatio) const;
  void queueRasterJobs(const std::shared_ptr<const DiffRasterSnapshot>& snapshot,
                       const std::vector<TileSpec>& specs,
                       int priority);
  void acceptRasteredTile(quint64 generation, quint64 key, QImage image);
  quint64 tileContentKey() const;
  quint64 tileGeometryKey(const QString& mode,
                          qreal contentWidth,
                          qreal visibleWidth,
                          qreal visibleHeight,
                          qreal unifiedRowWidth,
                          qreal splitTextLogicalWidth) const;
  quint64 tileGeometryKey(qreal visibleWidth,
                          qreal visibleHeight,
                          qreal unifiedRowWidth,
                          qreal splitTextLogicalWidth) const;
  void drawFileHeaderRow(QPainter* painter, const QRectF& rowRect, const DiffDisplayRow& row) const;
  void drawHunkRow(QPainter* painter, const QRectF& rowRect, const DiffDisplayRow& row) const;
  void drawUnifiedRow(QPainter* painter, const QRectF& rowRect, const DiffDisplayRow& row, bool selected) const;
  void drawSplitPaneFixedRow(QPainter* painter,
                             const QRectF& rowRect,
                             const DiffDisplayRow& row,
                             bool isLeftPane,
                             bool selected) const;
  void drawSplitPaneTextRow(QPainter* painter,
                            const QRectF& rowRect,
                            const DiffDisplayRow& row,
                            bool isLeftPane) const;
  void drawSplitRow(QPainter* painter,
                    const QRectF& rowRect,
                    const DiffDisplayRow& row,
                    bool selected,
                    qreal leftViewportX,
                    qreal rightViewportX) const;
  void drawTextRunWrapped(QPainter* painter,
                         const QPointF& baseline,
                         const QRectF& clipRect,
                         const QString& text,
                         const std::vector<DiffTokenSpan>& tokens,
                         const std::vector<DiffTokenSpan>& changeSpans,
                         const std::vector<qreal>& prefixAdvances,
                         const QColor& textColor,
                         const QColor& tokenBackground,
                         const QFontMetricsF& metrics) const;
  void drawTextRun(QPainter* painter,
                   const QPointF& baseline,
                   const QRectF& clipRect,
                   const QString& text,
                   const std::vector<DiffTokenSpan>& tokens,
                   const std::vector<DiffTokenSpan>& changeSpans,
                   const std::vector<qreal>& prefixAdvances,
                   const QColor& textColor,
                   const QColor& tokenBackground) const;

 protected:
  QSGNode* updatePaintNode(QSGNode* oldNode, UpdatePaintNodeData* data) override;
  void releaseResources() override;
  void mousePressEvent(QMouseEvent* event) override;
  void mouseMoveEvent(QMouseEvent* event) override;
  void mouseReleaseEvent(QMouseEvent* event) override;
  void wheelEvent(QWheelEvent* event) override;
  void hoverMoveEvent(QHoverEvent* event) override;
  void hoverLeaveEvent(QHoverEvent* event) override;
  void keyPressEvent(QKeyEvent* event) override;

 private:
  QObject* rowsModelObject_ = nullptr;
  DiffRowListModel* rowsModel_ = nullptr;
  QString layoutMode_ = "unified";
  int compareGeneration_ = 0;
  QString filePath_;
  QString fileStatus_ = "M";
  int additions_ = 0;
  int deletions_ = 0;
  QVariantMap palette_;
  QString monoFontFamily_ = "JetBrains Mono";

  TextRope textRope_;
  DiffLayoutEngine displayModel_;

  qreal contentHeight_ = 0;
  qreal contentWidth_ = 0;
  qreal viewportX_ = 0;
  qreal viewportY_ = 0;
  qreal leftViewportX_ = 0;
  qreal rightViewportX_ = 0;
  qreal viewportHeight_ = 0;
  qreal lineHeight_ = 0;
  qreal rowHeight_ = 0;
  qreal fileHeaderHeight_ = 28;
  qreal hunkHeight_ = 24;
  int lineNumberDigits_ = 3;
  bool wrapEnabled_ = false;
  int wrapColumn_ = 0;
  qreal maxTextWidth_ = 0;

  int paintCount_ = 0;
  int selectionAnchorRow_ = -1;
  int selectionCursorRow_ = -1;
  int hoveredRow_ = -1;
  int firstVisibleRow_ = -1;
  int lastVisibleRow_ = -1;
  int stickyVisibleRow_ = -1;

  mutable QHash<quint64, QString> textCache_;
  mutable QHash<LineLayoutCacheKey, CachedLineLayout> lineLayoutCache_;
  mutable QHash<LineLayoutCacheKey, quint64> lineLayoutLastUsed_;
  mutable quint64 lineLayoutUseTick_ = 0;
  mutable QHash<WrappedLineLayoutCacheKey, CachedWrappedLayout> wrappedLayoutCache_;
  mutable QHash<WrappedLineLayoutCacheKey, quint64> wrappedLayoutLastUsed_;
  mutable quint64 wrappedLayoutUseTick_ = 0;
  mutable QHash<quint64, QImage> tileImageCache_;
  mutable QHash<quint64, quint64> tileImageLastUsed_;
  mutable QHash<quint64, QSGTexture*> residentTextureCache_;
  mutable QHash<quint64, quint64> residentTextureLastUsed_;
  quint64 tilePaletteGeneration_ = 1;
  quint64 tileUseTick_ = 0;
  int tileCacheHits_ = 0;
  int tileCacheMisses_ = 0;
  int textureUploadCount_ = 0;
  int pendingTileJobCount_ = 0;
  double lastPaintTimeMs_ = 0;
  double lastRasterTimeMs_ = 0;
  double lastTextureUploadTimeMs_ = 0;
  double lastRowsRebuildTimeMs_ = 0;
  double lastDisplayRowsRebuildTimeMs_ = 0;
  double lastMetricsRecalcTimeMs_ = 0;
  bool followupUpdateQueued_ = false;
  bool rowsRebuildQueued_ = false;
  bool metricsRecalcQueued_ = false;
  bool alternateLayoutPrewarmQueued_ = false;
  bool currentTileRasterQueued_ = false;
  bool alternateTilePrewarmQueued_ = false;
  bool viewportJumpFallbackArmed_ = false;
  quint64 rasterGeneration_ = 1;
  QThreadPool rasterThreadPool_;
  mutable std::mutex rasterJobStateMutex_;
  QSet<quint64> pendingRasterKeys_;
  mutable std::mutex readyTileImagesMutex_;
  mutable QHash<quint64, QImage> readyTileImages_;
};

}  // namespace diffy
