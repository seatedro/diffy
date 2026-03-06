#pragma once

#include <QAbstractListModel>
#include <QVector>

#include "core/DiffTypes.h"

namespace diffy {

struct FlattenedDiffRow {
  enum class RowType {
    Hunk,
    Line,
  };

  RowType rowType = RowType::Line;
  int hunkIndex = -1;
  QString header;
  LineKind kind = LineKind::Context;
  int oldLine = -1;
  int newLine = -1;
  QString text;
  std::vector<TokenSpan> tokens;
  std::vector<TokenSpan> changeSpans;
};

std::vector<FlattenedDiffRow> flattenFileRows(const FileDiff& file);

class DiffRowListModel : public QAbstractListModel {
  Q_OBJECT
  Q_PROPERTY(int count READ count NOTIFY countChanged)

 public:
  enum Role {
    RowTypeRole = Qt::UserRole + 1,
    HunkIndexRole,
    HeaderRole,
    KindRole,
    OldLineRole,
    NewLineRole,
    TextRole,
    TokenCountRole,
  };

  explicit DiffRowListModel(QObject* parent = nullptr);

  int rowCount(const QModelIndex& parent = QModelIndex()) const override;
  QVariant data(const QModelIndex& index, int role) const override;
  QHash<int, QByteArray> roleNames() const override;

  int count() const;
  void clear();
  void setRows(std::vector<FlattenedDiffRow> rows);

  const std::vector<FlattenedDiffRow>& rows() const;

 signals:
  void countChanged();

 private:
  std::vector<FlattenedDiffRow> rows_;
};

}  // namespace diffy
