#pragma once

#include <QHash>
#include <QHoverEvent>
#include <QKeyEvent>
#include <QMouseEvent>
#include <QQuickPaintedItem>
#include <QVariantList>
#include <QVariantMap>

#include "model/DiffDisplayModel.h"
#include "text/TextRope.h"

namespace diffy {

class DiffSurfaceItem : public QQuickPaintedItem {
  Q_OBJECT
  Q_PROPERTY(QVariantList rowsModel READ rowsModel WRITE setRowsModel NOTIFY rowsModelChanged)
  Q_PROPERTY(QString layoutMode READ layoutMode WRITE setLayoutMode NOTIFY layoutModeChanged)
  Q_PROPERTY(QVariantMap palette READ palette WRITE setPalette NOTIFY paletteChanged)
  Q_PROPERTY(QString monoFontFamily READ monoFontFamily WRITE setMonoFontFamily NOTIFY monoFontFamilyChanged)
  Q_PROPERTY(qreal contentHeight READ contentHeight NOTIFY contentHeightChanged)
  Q_PROPERTY(qreal contentWidth READ contentWidth NOTIFY contentWidthChanged)
  Q_PROPERTY(qreal viewportX READ viewportX WRITE setViewportX NOTIFY viewportXChanged)
  Q_PROPERTY(qreal viewportY READ viewportY WRITE setViewportY NOTIFY viewportYChanged)
  Q_PROPERTY(qreal viewportHeight READ viewportHeight WRITE setViewportHeight NOTIFY viewportHeightChanged)
  Q_PROPERTY(int paintCount READ paintCount NOTIFY paintCountChanged)
  Q_PROPERTY(int displayRowCount READ displayRowCount NOTIFY displayRowCountChanged)

 public:
  explicit DiffSurfaceItem(QQuickItem* parent = nullptr);

  QVariantList rowsModel() const;
  void setRowsModel(const QVariantList& rows);

  QString layoutMode() const;
  void setLayoutMode(const QString& mode);

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

  qreal viewportHeight() const;
  void setViewportHeight(qreal value);

  int paintCount() const;
  int displayRowCount() const;

  void paint(QPainter* painter) override;

 signals:
  void rowsModelChanged();
  void layoutModeChanged();
  void paletteChanged();
  void monoFontFamilyChanged();
  void contentHeightChanged();
  void contentWidthChanged();
  void viewportXChanged();
  void viewportYChanged();
  void viewportHeightChanged();
  void paintCountChanged();
  void displayRowCountChanged();

 private:
  void rebuildRows();
  void rebuildDisplayRows();
  void recalculateMetrics();

  bool rowSelected(int rowIndex) const;
  QColor paletteColor(const QString& key, const QColor& fallback) const;
  qreal digitWidth() const;
  qreal unifiedGutterWidth() const;
  QString selectedText() const;
  QString textForRange(const TextRange& range) const;

  void drawHunkRow(QPainter* painter, const QRectF& rowRect, const DiffDisplayRow& row) const;
  void drawUnifiedRow(QPainter* painter, const QRectF& rowRect, const DiffDisplayRow& row, bool selected) const;
  void drawSplitRow(QPainter* painter, const QRectF& rowRect, const DiffDisplayRow& row, bool selected) const;
  void drawTextRun(QPainter* painter,
                   const QPointF& baseline,
                   const QRectF& clipRect,
                   const QString& text,
                   const std::vector<DiffTokenSpan>& tokens,
                   const QColor& textColor,
                   const QColor& tokenBackground) const;

 protected:
  void mousePressEvent(QMouseEvent* event) override;
  void mouseMoveEvent(QMouseEvent* event) override;
  void mouseReleaseEvent(QMouseEvent* event) override;
  void hoverMoveEvent(QHoverEvent* event) override;
  void hoverLeaveEvent(QHoverEvent* event) override;
  void keyPressEvent(QKeyEvent* event) override;

 private:
  QVariantList rowsModel_;
  QString layoutMode_ = "unified";
  QVariantMap palette_;
  QString monoFontFamily_ = "JetBrains Mono";

  TextRope textRope_;
  DiffDisplayModel displayModel_;

  qreal contentHeight_ = 0;
  qreal contentWidth_ = 0;
  qreal viewportX_ = 0;
  qreal viewportY_ = 0;
  qreal viewportHeight_ = 0;
  qreal lineHeight_ = 0;
  qreal rowHeight_ = 0;
  qreal hunkHeight_ = 24;
  int lineNumberDigits_ = 3;
  qreal maxTextWidth_ = 0;

  int paintCount_ = 0;
  int selectionAnchorRow_ = -1;
  int selectionCursorRow_ = -1;
  int hoveredRow_ = -1;
  int firstVisibleRow_ = -1;
  int lastVisibleRow_ = -1;
  int stickyVisibleRow_ = -1;

  mutable QHash<quint64, QString> textCache_;
};

}  // namespace diffy
