#include "core/rendering/FlatDiffRows.h"

namespace diffy {

std::vector<FlatDiffRow> flattenFileDiff(const FileDiff& file) {
  std::vector<FlatDiffRow> rows;
  rows.reserve(file.hunks.size());
  for (size_t hunkIndex = 0; hunkIndex < file.hunks.size(); ++hunkIndex) {
    const Hunk& hunk = file.hunks.at(hunkIndex);
    rows.push_back(FlatDiffRow{
        .rowType = FlatDiffRowType::Hunk,
        .hunkIndex = static_cast<int>(hunkIndex),
        .header = hunk.header,
    });

    for (const DiffLine& line : hunk.lines) {
      rows.push_back(FlatDiffRow{
          .rowType = FlatDiffRowType::Line,
          .hunkIndex = static_cast<int>(hunkIndex),
          .kind = line.kind,
          .oldLine = line.oldLine,
          .newLine = line.newLine,
          .text = line.text,
          .tokens = line.tokens,
          .changeSpans = line.changeSpans,
      });
    }
  }
  return rows;
}

}  // namespace diffy
