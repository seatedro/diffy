#pragma once

#include <string>
#include <vector>

#include "core/diff/DiffTypes.h"

namespace diffy {

enum class FlatDiffRowType {
  Hunk,
  Line,
};

struct FlatDiffRow {
  FlatDiffRowType rowType = FlatDiffRowType::Line;
  int hunkIndex = -1;
  std::string header;
  LineKind kind = LineKind::Context;
  int oldLine = -1;
  int newLine = -1;
  std::string text;
  std::vector<TokenSpan> tokens;
  std::vector<TokenSpan> changeSpans;
};

std::vector<FlatDiffRow> flattenFileDiff(const FileDiff& file);

}  // namespace diffy
