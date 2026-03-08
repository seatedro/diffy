#include "core/rendering/PreparedRows.h"

#include <algorithm>

namespace diffy {
std::vector<DiffTokenSpan> convertTokens(const std::vector<TokenSpan>& tokenValues) {
  std::vector<DiffTokenSpan> tokens;
  tokens.reserve(tokenValues.size());
  for (const TokenSpan& tokenValue : tokenValues) {
    tokens.push_back(DiffTokenSpan{tokenValue.start, tokenValue.length, tokenValue.syntaxKind});
  }
  return tokens;
}

PreparedRows prepareRowsForDisplay(const std::vector<FlatDiffRow>& rows, const TextWidthMeasure& measureTextWidth) {
  PreparedRows prepared;
  prepared.sourceRows.reserve(rows.size());

  for (const FlatDiffRow& rowValue : rows) {
    DiffSourceRow row;
    row.rowType = rowValue.rowType == FlatDiffRowType::Hunk ? DiffRowType::Hunk : DiffRowType::Line;
    row.header = rowValue.header;
    row.kind = rowValue.kind == LineKind::Addition
                   ? DiffLineKind::Addition
                   : rowValue.kind == LineKind::Deletion ? DiffLineKind::Deletion : DiffLineKind::Context;
    row.oldLine = rowValue.oldLine;
    row.newLine = rowValue.newLine;
    row.textRange = prepared.textRope.append(rowValue.text);
    row.textWidth = measureTextWidth != nullptr ? measureTextWidth(rowValue.text)
                                                : static_cast<double>(rowValue.text.size());
    row.tokens = convertTokens(rowValue.tokens);
    row.changeSpans = convertTokens(rowValue.changeSpans);
    prepared.maxTextWidth = std::max(prepared.maxTextWidth, row.textWidth);
    prepared.sourceRows.push_back(std::move(row));
  }

  return prepared;
}

}  // namespace diffy
