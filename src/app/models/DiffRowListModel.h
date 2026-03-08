#pragma once

#include <QAbstractListModel>

#include "core/rendering/FlatDiffRows.h"
#include "core/rendering/PreparedRows.h"

namespace diffy {

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
  void setRows(std::vector<FlatDiffRow> rows);
  void clearPreparedRows();
  const PreparedRows* preparedRows(const PreparedRowsCacheKey& key) const;
  void storePreparedRows(PreparedRowsCacheKey key, PreparedRows prepared);

  const std::vector<FlatDiffRow>& rows() const;

 signals:
  void countChanged();

 private:
  std::vector<FlatDiffRow> rows_;
  QHash<PreparedRowsCacheKey, PreparedRows> preparedRowsCache_;
};

}  // namespace diffy
