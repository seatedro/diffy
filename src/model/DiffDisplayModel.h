#pragma once

#include <string>
#include <vector>

#include "text/TextRope.h"

namespace diffy {

enum class DiffRowType {
  FileHeader,
  Hunk,
  Line,
};

enum class DiffLineKind {
  Context,
  Addition,
  Deletion,
  Spacer,
  Change,
};

enum class DiffLayoutMode {
  Unified,
  Split,
};

struct DiffTokenSpan {
  int start = 0;
  int length = 0;
};

struct DiffSourceRow {
  DiffRowType rowType = DiffRowType::Line;
  std::string header;
  std::string detail;
  DiffLineKind kind = DiffLineKind::Context;
  int oldLine = -1;
  int newLine = -1;
  std::vector<DiffTokenSpan> tokens;
  TextRange textRange;
};

struct DiffDisplayRow {
  DiffRowType rowType = DiffRowType::Line;
  std::string header;
  std::string detail;
  DiffLineKind kind = DiffLineKind::Context;
  int oldLine = -1;
  int newLine = -1;
  std::vector<DiffTokenSpan> tokens;
  DiffLineKind leftKind = DiffLineKind::Spacer;
  DiffLineKind rightKind = DiffLineKind::Spacer;
  int leftLine = -1;
  int rightLine = -1;
  std::vector<DiffTokenSpan> leftTokens;
  std::vector<DiffTokenSpan> rightTokens;
  TextRange textRange;
  TextRange leftTextRange;
  TextRange rightTextRange;
  double top = 0;
  double height = 0;
};

class DiffDisplayModel {
 public:
  void setSourceRows(std::vector<DiffSourceRow> rows);
  void rebuild(DiffLayoutMode mode, double rowHeight, double hunkHeight, double fileHeaderHeight);

  const std::vector<DiffDisplayRow>& rows() const;
  double contentHeight() const;
  int lineNumberDigits() const;

  int rowIndexAtY(double y) const;
  int fileHeaderRowIndex() const;
  int stickyHunkRowIndexAtY(double y) const;
  int nextHunkRowIndex(int rowIndex) const;
  int previousHunkRowIndex(int rowIndex) const;

 private:
  std::vector<DiffSourceRow> sourceRows_;
  std::vector<DiffDisplayRow> displayRows_;
  std::vector<double> rowOffsets_;
  double contentHeight_ = 0;
  int lineNumberDigits_ = 3;
};

}  // namespace diffy
