#include "model/DiffRowListModel.h"

namespace diffy {
namespace {

QString rowTypeToString(FlattenedDiffRow::RowType rowType) {
  return rowType == FlattenedDiffRow::RowType::Hunk ? "hunk" : "line";
}

}  // namespace

QVector<FlattenedDiffRow> flattenFileRows(const FileDiff& file) {
  QVector<FlattenedDiffRow> rows;
  for (qsizetype hunkIndex = 0; hunkIndex < file.hunks.size(); ++hunkIndex) {
    const Hunk& hunk = file.hunks.at(hunkIndex);
    rows.push_back(FlattenedDiffRow{
        .rowType = FlattenedDiffRow::RowType::Hunk,
        .hunkIndex = static_cast<int>(hunkIndex),
        .header = hunk.header,
    });

    for (const DiffLine& line : hunk.lines) {
      rows.push_back(FlattenedDiffRow{
          .rowType = FlattenedDiffRow::RowType::Line,
          .hunkIndex = static_cast<int>(hunkIndex),
          .kind = line.kind,
          .oldLine = line.oldLine,
          .newLine = line.newLine,
          .text = line.text,
          .tokens = line.tokens,
      });
    }
  }
  return rows;
}

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

  const FlattenedDiffRow& row = rows_.at(index.row());
  switch (role) {
    case RowTypeRole:
      return rowTypeToString(row.rowType);
    case HunkIndexRole:
      return row.hunkIndex;
    case HeaderRole:
      return row.header;
    case KindRole:
      return lineKindToString(row.kind);
    case OldLineRole:
      return row.oldLine;
    case NewLineRole:
      return row.newLine;
    case TextRole:
      return row.text;
    case TokenCountRole:
      return row.tokens.size();
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

void DiffRowListModel::setRows(QVector<FlattenedDiffRow> rows) {
  const int previousCount = rows_.size();
  beginResetModel();
  rows_ = std::move(rows);
  endResetModel();
  if (previousCount != rows_.size()) {
    emit countChanged();
  }
}

const QVector<FlattenedDiffRow>& DiffRowListModel::rows() const {
  return rows_;
}

}  // namespace diffy
