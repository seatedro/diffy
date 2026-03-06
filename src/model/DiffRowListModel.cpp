#include "model/DiffRowListModel.h"

namespace diffy {
namespace {

QString rowTypeToString(FlattenedDiffRow::RowType rowType) {
  return rowType == FlattenedDiffRow::RowType::Hunk ? "hunk" : "line";
}

}  // namespace

std::vector<FlattenedDiffRow> flattenFileRows(const FileDiff& file) {
  std::vector<FlattenedDiffRow> rows;
  rows.reserve(file.hunks.size());
  for (size_t hunkIndex = 0; hunkIndex < file.hunks.size(); ++hunkIndex) {
    const Hunk& hunk = file.hunks.at(hunkIndex);
    rows.push_back(FlattenedDiffRow{
        .rowType = FlattenedDiffRow::RowType::Hunk,
        .hunkIndex = static_cast<int>(hunkIndex),
        .header = QString::fromUtf8(hunk.header),
    });

    for (const DiffLine& line : hunk.lines) {
      rows.push_back(FlattenedDiffRow{
          .rowType = FlattenedDiffRow::RowType::Line,
          .hunkIndex = static_cast<int>(hunkIndex),
          .kind = line.kind,
          .oldLine = line.oldLine,
          .newLine = line.newLine,
          .text = QString::fromUtf8(line.text),
          .tokens = line.tokens,
          .changeSpans = line.changeSpans,
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
      return QString::fromLatin1(lineKindToString(row.kind).data(),
                                 static_cast<int>(lineKindToString(row.kind).size()));
    case OldLineRole:
      return row.oldLine;
    case NewLineRole:
      return row.newLine;
    case TextRole:
      return row.text;
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

void DiffRowListModel::setRows(std::vector<FlattenedDiffRow> rows) {
  const int previousCount = static_cast<int>(rows_.size());
  beginResetModel();
  rows_ = std::move(rows);
  endResetModel();
  if (previousCount != static_cast<int>(rows_.size())) {
    emit countChanged();
  }
}

const std::vector<FlattenedDiffRow>& DiffRowListModel::rows() const {
  return rows_;
}

}  // namespace diffy
