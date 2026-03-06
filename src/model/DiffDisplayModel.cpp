#include "model/DiffDisplayModel.h"

#include <algorithm>

namespace diffy {

void DiffDisplayModel::setSourceRows(std::vector<DiffSourceRow> rows) {
  sourceRows_ = std::move(rows);
}

void DiffDisplayModel::rebuild(DiffLayoutMode mode, double rowHeight, double hunkHeight, double fileHeaderHeight) {
  displayRows_.clear();
  rowOffsets_.clear();

  int maxLineNumber = 0;
  double top = 0;

  auto appendRow = [&](DiffDisplayRow row) {
    row.top = top;
    row.height = row.rowType == DiffRowType::FileHeader ? fileHeaderHeight
               : row.rowType == DiffRowType::Hunk         ? hunkHeight
                                                         : rowHeight;
    rowOffsets_.push_back(top);
    top += row.height;
    maxLineNumber = std::max(maxLineNumber, std::max(row.oldLine, row.newLine));
    maxLineNumber = std::max(maxLineNumber, std::max(row.leftLine, row.rightLine));
    displayRows_.push_back(std::move(row));
  };

  if (mode == DiffLayoutMode::Unified) {
    for (const DiffSourceRow& sourceRow : sourceRows_) {
      DiffDisplayRow row;
      row.rowType = sourceRow.rowType;
      row.header = sourceRow.header;
      row.detail = sourceRow.detail;
      row.kind = sourceRow.kind;
      row.oldLine = sourceRow.oldLine;
      row.newLine = sourceRow.newLine;
      row.tokens = sourceRow.tokens;
      row.textRange = sourceRow.textRange;
      appendRow(std::move(row));
    }
  } else {
    for (size_t index = 0; index < sourceRows_.size(); ++index) {
      const DiffSourceRow& sourceRow = sourceRows_.at(index);
      if (sourceRow.rowType == DiffRowType::FileHeader || sourceRow.rowType == DiffRowType::Hunk) {
        DiffDisplayRow row;
        row.rowType = sourceRow.rowType;
        row.header = sourceRow.header;
        row.detail = sourceRow.detail;
        appendRow(std::move(row));
        continue;
      }

      if (sourceRow.kind == DiffLineKind::Context) {
        DiffDisplayRow row;
        row.rowType = DiffRowType::Line;
        row.kind = DiffLineKind::Context;
        row.oldLine = sourceRow.oldLine;
        row.newLine = sourceRow.newLine;
        row.leftKind = DiffLineKind::Context;
        row.rightKind = DiffLineKind::Context;
        row.leftLine = sourceRow.oldLine;
        row.rightLine = sourceRow.newLine;
        row.leftTokens = sourceRow.tokens;
        row.rightTokens = sourceRow.tokens;
        row.leftTextRange = sourceRow.textRange;
        row.rightTextRange = sourceRow.textRange;
        appendRow(std::move(row));
        continue;
      }

      std::vector<DiffSourceRow> deletions;
      std::vector<DiffSourceRow> additions;

      while (index < sourceRows_.size()) {
        const DiffSourceRow& blockRow = sourceRows_.at(index);
        if (blockRow.rowType != DiffRowType::Line || blockRow.kind == DiffLineKind::Context) {
          --index;
          break;
        }

        if (blockRow.kind == DiffLineKind::Deletion) {
          deletions.push_back(blockRow);
        } else if (blockRow.kind == DiffLineKind::Addition) {
          additions.push_back(blockRow);
        }
        ++index;
      }

      const size_t rowCount = std::max(deletions.size(), additions.size());
      for (size_t rowIndex = 0; rowIndex < rowCount; ++rowIndex) {
        DiffDisplayRow row;
        row.rowType = DiffRowType::Line;
        row.kind = DiffLineKind::Change;

        if (rowIndex < deletions.size()) {
          const DiffSourceRow& left = deletions.at(rowIndex);
          row.leftKind = DiffLineKind::Deletion;
          row.leftLine = left.oldLine;
          row.leftTokens = left.tokens;
          row.leftTextRange = left.textRange;
          row.oldLine = left.oldLine;
        }

        if (rowIndex < additions.size()) {
          const DiffSourceRow& right = additions.at(rowIndex);
          row.rightKind = DiffLineKind::Addition;
          row.rightLine = right.newLine;
          row.rightTokens = right.tokens;
          row.rightTextRange = right.textRange;
          row.newLine = right.newLine;
        }

        appendRow(std::move(row));
      }
    }
  }

  contentHeight_ = top;
  lineNumberDigits_ = std::max(3, static_cast<int>(std::to_string(std::max(0, maxLineNumber)).size()));
}

const std::vector<DiffDisplayRow>& DiffDisplayModel::rows() const {
  return displayRows_;
}

double DiffDisplayModel::contentHeight() const {
  return contentHeight_;
}

int DiffDisplayModel::lineNumberDigits() const {
  return lineNumberDigits_;
}

int DiffDisplayModel::rowIndexAtY(double y) const {
  if (displayRows_.empty()) {
    return -1;
  }

  const auto it = std::upper_bound(rowOffsets_.cbegin(), rowOffsets_.cend(), y);
  if (it == rowOffsets_.cbegin()) {
    return 0;
  }
  return std::clamp(static_cast<int>(std::distance(rowOffsets_.cbegin(), it) - 1), 0,
                    static_cast<int>(displayRows_.size() - 1));
}

int DiffDisplayModel::fileHeaderRowIndex() const {
  for (int rowIndex = 0; rowIndex < static_cast<int>(displayRows_.size()); ++rowIndex) {
    if (displayRows_.at(rowIndex).rowType == DiffRowType::FileHeader) {
      return rowIndex;
    }
  }
  return -1;
}

int DiffDisplayModel::stickyHunkRowIndexAtY(double y) const {
  int stickyIndex = -1;
  for (int rowIndex = 0; rowIndex < static_cast<int>(displayRows_.size()); ++rowIndex) {
    const DiffDisplayRow& row = displayRows_.at(rowIndex);
    if (row.rowType == DiffRowType::Hunk && row.top <= y) {
      stickyIndex = rowIndex;
    }
    if (row.top > y) {
      break;
    }
  }
  return stickyIndex;
}

int DiffDisplayModel::nextHunkRowIndex(int rowIndex) const {
  const int start = std::max(0, rowIndex + 1);
  for (int index = start; index < static_cast<int>(displayRows_.size()); ++index) {
    if (displayRows_.at(index).rowType == DiffRowType::Hunk) {
      return index;
    }
  }
  return -1;
}

int DiffDisplayModel::previousHunkRowIndex(int rowIndex) const {
  const int start = std::min(rowIndex - 1, static_cast<int>(displayRows_.size()) - 1);
  for (int index = start; index >= 0; --index) {
    if (displayRows_.at(index).rowType == DiffRowType::Hunk) {
      return index;
    }
  }
  return -1;
}

}  // namespace diffy
