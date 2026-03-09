#include "core/rendering/PreparedRows.h"

#include <algorithm>

namespace diffy {

TokenRange appendConvertedTokens(TokenBuffer& buf, const std::vector<TokenSpan>& tokenValues) {
  if (tokenValues.empty()) {
    return {};
  }
  const uint32_t start = static_cast<uint32_t>(buf.size());
  for (const TokenSpan& tv : tokenValues) {
    DiffTokenSpan span{tv.start, tv.length, tv.syntaxKind};
    buf.append(&span, 1);
  }
  return {start, static_cast<uint32_t>(tokenValues.size())};
}

PreparedRows prepareRowsForDisplay(const std::vector<FlatDiffRow>& rows, const TextWidthMeasure& measureTextWidth) {
  PreparedRows prepared;
  prepared.sourceRows.reserve(rows.size());
  size_t totalTextBytes = 0;
  size_t totalTokens = 0;
  for (const FlatDiffRow& rowValue : rows) {
    totalTextBytes += rowValue.text.size();
    totalTokens += rowValue.tokens.size() + rowValue.changeSpans.size();
  }
  prepared.textBuffer.reserve(totalTextBytes);
  prepared.tokenBuffer.reserve(totalTokens);

  for (const FlatDiffRow& rowValue : rows) {
    DiffSourceRow row;
    row.rowType = rowValue.rowType == FlatDiffRowType::Hunk ? DiffRowType::Hunk : DiffRowType::Line;
    row.header = rowValue.header;
    row.kind = rowValue.kind == LineKind::Addition
                   ? DiffLineKind::Addition
                   : rowValue.kind == LineKind::Deletion ? DiffLineKind::Deletion : DiffLineKind::Context;
    row.oldLine = rowValue.oldLine;
    row.newLine = rowValue.newLine;
    row.textRange = prepared.textBuffer.append(rowValue.text);
    row.textWidth = measureTextWidth != nullptr ? measureTextWidth(rowValue.text)
                                                : static_cast<double>(rowValue.text.size());
    row.tokens = appendConvertedTokens(prepared.tokenBuffer, rowValue.tokens);
    row.changeSpans = appendConvertedTokens(prepared.tokenBuffer, rowValue.changeSpans);
    prepared.maxTextWidth = std::max(prepared.maxTextWidth, row.textWidth);
    prepared.sourceRows.push_back(std::move(row));
  }

  return prepared;
}

}  // namespace diffy
