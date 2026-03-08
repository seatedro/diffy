#include "app/models/DiffRowListModel.h"

namespace diffy {
namespace {

QString rowTypeToString(FlatDiffRowType rowType) {
  return rowType == FlatDiffRowType::Hunk ? "hunk" : "line";
}

}  // namespace

DiffRowListModel::DiffRowListModel(QObject* parent) : QAbstractListModel(parent) {}

int DiffRowListModel::rowCount(const QModelIndex& parent) const {
  if (parent.isValid()) {
    return 0;
  }
  return rows_.size();
}

QVariant DiffRowListModel::data(const QModelIndex& index, int role) const {
  if (!index.isValid() || index.row() < 0 || index.row() >= rows_.size()) {
    return {};
  }

  const FlatDiffRow& row = rows_.at(index.row());
  switch (role) {
    case RowTypeRole:
      return rowTypeToString(row.rowType);
    case HunkIndexRole:
      return row.hunkIndex;
    case HeaderRole:
      return QString::fromStdString(row.header);
    case KindRole:
      return QString::fromLatin1(lineKindToString(row.kind).data(),
                                 static_cast<int>(lineKindToString(row.kind).size()));
    case OldLineRole:
      return row.oldLine;
    case NewLineRole:
      return row.newLine;
    case TextRole:
      return QString::fromStdString(row.text);
    case TokenCountRole:
      return static_cast<int>(row.tokens.size());
    default:
      return {};
  }
}

QHash<int, QByteArray> DiffRowListModel::roleNames() const {
  return {
      {RowTypeRole, "rowType"},
      {HunkIndexRole, "hunkIndex"},
      {HeaderRole, "header"},
      {KindRole, "kind"},
      {OldLineRole, "oldLine"},
      {NewLineRole, "newLine"},
      {TextRole, "text"},
      {TokenCountRole, "tokenCount"},
  };
}

int DiffRowListModel::count() const {
  return rowCount();
}

void DiffRowListModel::clear() {
  setRows({});
}

void DiffRowListModel::setRows(std::vector<FlatDiffRow> rows) {
  const int previousCount = static_cast<int>(rows_.size());
  beginResetModel();
  rows_ = std::move(rows);
  endResetModel();
  if (previousCount != static_cast<int>(rows_.size())) {
    emit countChanged();
  }
}

void DiffRowListModel::clearPreparedRows() {
  preparedRowsCache_.clear();
}

const PreparedRows* DiffRowListModel::preparedRows(const PreparedRowsCacheKey& key) const {
  if (const auto it = preparedRowsCache_.constFind(key); it != preparedRowsCache_.constEnd()) {
    return &it.value();
  }
  return nullptr;
}

void DiffRowListModel::storePreparedRows(PreparedRowsCacheKey key, PreparedRows prepared) {
  preparedRowsCache_.insert(std::move(key), std::move(prepared));
  while (preparedRowsCache_.size() > 64) {
    preparedRowsCache_.erase(preparedRowsCache_.begin());
  }
}

const std::vector<FlatDiffRow>& DiffRowListModel::rows() const {
  return rows_;
}

}  // namespace diffy
