#pragma once

#include <QQuickPaintedItem>
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
  void paintCountChanged();
  void displayRowCountChanged();

 private:
  void rebuildRows();
  void rebuildDisplayRows();
  void recalculateMetrics();
  int rowIndexAtY(qreal y) const;
  QColor paletteColor(const QString& key, const QColor& fallback) const;
  qreal digitWidth() const;
  qreal unifiedGutterWidth() const;

  void drawHunkRow(QPainter* painter, const QRectF& rowRect, const Row& row) const;
  void drawUnifiedRow(QPainter* painter, const QRectF& rowRect, const Row& row) const;
  void drawSplitRow(QPainter* painter, const QRectF& rowRect, const Row& row) const;
  void drawTextRun(QPainter* painter,
                   const QPointF& baseline,
                   const QRectF& clipRect,
                   const QString& text,
                   const QVector<TokenSpan>& tokens,
                   const QColor& textColor,
                   const QColor& tokenBackground) const;

  QVariantList rowsModel_;
  QString layoutMode_ = "unified";
  QVariantMap palette_;
  QString monoFontFamily_ = "JetBrains Mono";

  QVector<Row> sourceRows_;
  QVector<Row> displayRows_;
  QVector<qreal> rowOffsets_;

  qreal contentHeight_ = 0;
  qreal contentWidth_ = 0;
  qreal lineHeight_ = 0;
  qreal rowHeight_ = 0;
  qreal hunkHeight_ = 24;
  int lineNumberDigits_ = 3;
  qreal maxTextWidth_ = 0;
  int paintCount_ = 0;
};

}  // namespace diffy
