#pragma once

#include <QQuickPaintedItem>
#include <QMouseEvent>
#include <QKeyEvent>
#include <QHoverEvent>
#include <QVariantList>
#include <QVariantMap>

namespace diffy {

class DiffSurfaceItem : public QQuickPaintedItem {
  Q_OBJECT
  Q_PROPERTY(QVariantList rowsModel READ rowsModel WRITE setRowsModel NOTIFY rowsModelChanged)
  Q_PROPERTY(QString layoutMode READ layoutMode WRITE setLayoutMode NOTIFY layoutModeChanged)
  Q_PROPERTY(QVariantMap palette READ palette WRITE setPalette NOTIFY paletteChanged)
  Q_PROPERTY(QString monoFontFamily READ monoFontFamily WRITE setMonoFontFamily NOTIFY monoFontFamilyChanged)
  Q_PROPERTY(qreal contentHeight READ contentHeight NOTIFY contentHeightChanged)
  Q_PROPERTY(qreal contentWidth READ contentWidth NOTIFY contentWidthChanged)
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
  qreal viewportY() const;
  void setViewportY(qreal value);

  qreal viewportHeight() const;
  void setViewportHeight(qreal value);

  int paintCount() const;
  int displayRowCount() const;

  void paint(QPainter* painter) override;

 public:
  struct TokenSpan {
    int start = 0;
    int length = 0;
  };

  struct Row {
    QString rowType;
    QString header;
    QString kind;
    int oldLine = -1;
    int newLine = -1;
    QString text;
    QVector<TokenSpan> tokens;
    QString leftKind;
    QString rightKind;
    int leftLine = -1;
    int rightLine = -1;
    QString leftText;
    QString rightText;
    QVector<TokenSpan> leftTokens;
    QVector<TokenSpan> rightTokens;
    qreal top = 0;
    qreal height = 0;
  };

 signals:
  void rowsModelChanged();
  void layoutModeChanged();
  void paletteChanged();
  void monoFontFamilyChanged();
  void contentHeightChanged();
  void contentWidthChanged();
  void viewportYChanged();
  void viewportHeightChanged();
  void paintCountChanged();
  void displayRowCountChanged();

 private:
  void rebuildRows();
  void rebuildDisplayRows();
  void recalculateMetrics();
  int rowIndexAtY(qreal y) const;
  bool rowSelected(int rowIndex) const;
  QColor paletteColor(const QString& key, const QColor& fallback) const;
  qreal digitWidth() const;
  qreal unifiedGutterWidth() const;
  QString selectedText() const;

  void drawHunkRow(QPainter* painter, const QRectF& rowRect, const Row& row) const;
  void drawUnifiedRow(QPainter* painter, const QRectF& rowRect, const Row& row, bool selected) const;
  void drawSplitRow(QPainter* painter, const QRectF& rowRect, const Row& row, bool selected) const;
  void drawTextRun(QPainter* painter,
                   const QPointF& baseline,
                   const QRectF& clipRect,
                   const QString& text,
                   const QVector<TokenSpan>& tokens,
                   const QColor& textColor,
                   const QColor& tokenBackground) const;

 protected:
  void mousePressEvent(QMouseEvent* event) override;
  void mouseMoveEvent(QMouseEvent* event) override;
  void mouseReleaseEvent(QMouseEvent* event) override;
  void hoverMoveEvent(QHoverEvent* event) override;
  void hoverLeaveEvent(QHoverEvent* event) override;
  void keyPressEvent(QKeyEvent* event) override;

  QVariantList rowsModel_;
  QString layoutMode_ = "unified";
  QVariantMap palette_;
  QString monoFontFamily_ = "JetBrains Mono";

  QVector<Row> sourceRows_;
  QVector<Row> displayRows_;
  QVector<qreal> rowOffsets_;

  qreal contentHeight_ = 0;
  qreal contentWidth_ = 0;
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
};

}  // namespace diffy
