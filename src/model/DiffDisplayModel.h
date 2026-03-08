#pragma once

#include <optional>
#include <string>
#include <vector>

#include "core/SyntaxTypes.h"
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
  SyntaxTokenKind syntaxKind = SyntaxTokenKind::None;
};

struct DiffSourceRow {
  DiffRowType rowType = DiffRowType::Line;
  std::string header;
  std::string detail;
  DiffLineKind kind = DiffLineKind::Context;
  int oldLine = -1;
  int newLine = -1;
  double textWidth = 0;
  std::vector<DiffTokenSpan> tokens;
  std::vector<DiffTokenSpan> changeSpans;
  TextRange textRange;
};

struct DiffLayoutConfig {
  DiffLayoutMode mode = DiffLayoutMode::Unified;
  double rowHeight = 0;
  double hunkHeight = 0;
  double fileHeaderHeight = 0;
  bool wrapEnabled = false;
  double unifiedWrapWidth = 0;
  double splitWrapWidth = 0;
};

struct DiffDisplayRow {
  DiffRowType rowType = DiffRowType::Line;
  std::string header;
  std::string detail;
  double textWidth = 0;
  DiffLineKind kind = DiffLineKind::Context;
  int oldLine = -1;
  int newLine = -1;
  std::vector<DiffTokenSpan> tokens;
  std::vector<DiffTokenSpan> changeSpans;
  DiffLineKind leftKind = DiffLineKind::Spacer;
  DiffLineKind rightKind = DiffLineKind::Spacer;
  int leftLine = -1;
  int rightLine = -1;
  std::vector<DiffTokenSpan> leftTokens;
  std::vector<DiffTokenSpan> leftChangeSpans;
  std::vector<DiffTokenSpan> rightTokens;
  std::vector<DiffTokenSpan> rightChangeSpans;
  TextRange textRange;
  TextRange leftTextRange;
  TextRange rightTextRange;
  double top = 0;
  double height = 0;
};

class DiffDisplayModel {
 public:
  void setFileHeader(std::optional<DiffSourceRow> row);
  void setSourceRows(std::vector<DiffSourceRow> rows);
  void rebuild(const DiffLayoutConfig& config);

  const std::vector<DiffDisplayRow>& rows() const;
  double contentHeight() const;
  int lineNumberDigits() const;

  int rowIndexAtY(double y) const;
  int fileHeaderRowIndex() const;
  int stickyHunkRowIndexAtY(double y) const;
  int nextHunkRowIndex(int rowIndex) const;
  int previousHunkRowIndex(int rowIndex) const;

 private:
  void recomputeLineNumberDigits();
  void clearTopologyCaches();
  void rebuildTopology(std::vector<DiffDisplayRow>& targetRows, DiffLayoutMode mode) const;
  void rebuildMetrics(const DiffLayoutConfig& config);

  std::optional<DiffSourceRow> fileHeaderRow_;
  std::vector<DiffSourceRow> sourceRows_;
  std::vector<DiffDisplayRow> unifiedTopologyRows_;
  std::vector<DiffDisplayRow> splitTopologyRows_;
  std::vector<DiffDisplayRow> displayRows_;
  std::vector<double> rowOffsets_;
  double contentHeight_ = 0;
  int lineNumberDigits_ = 3;
};

}  // namespace diffy
