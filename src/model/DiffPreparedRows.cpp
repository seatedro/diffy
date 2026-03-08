#include "model/DiffPreparedRows.h"

#include <QFont>
#include <QFontMetricsF>

#include <algorithm>

#include "model/DiffRowListModel.h"

namespace diffy {
namespace {

QFont monoFont(const QString& family, qreal pixelSize) {
  QFont font(family);
  font.setStyleHint(QFont::Monospace);
  font.setPixelSize(qRound(pixelSize));
  return font;
}

std::vector<DiffTokenSpan> convertTokens(const std::vector<TokenSpan>& tokenValues) {
  std::vector<DiffTokenSpan> tokens;
  tokens.reserve(tokenValues.size());
  for (const TokenSpan& tokenValue : tokenValues) {
    tokens.push_back(DiffTokenSpan{tokenValue.start, tokenValue.length, tokenValue.syntaxKind});
  }
  return tokens;
}

}  // namespace

PreparedRows prepareRowsForSurface(const std::vector<FlattenedDiffRow>& rows, const QString& monoFontFamily) {
  PreparedRows prepared;
  prepared.sourceRows.reserve(rows.size());

  const QFontMetricsF metrics(monoFont(monoFontFamily, 12));
  for (const FlattenedDiffRow& rowValue : rows) {
    DiffSourceRow row;
    row.rowType = rowValue.rowType == FlattenedDiffRow::RowType::Hunk ? DiffRowType::Hunk : DiffRowType::Line;
    row.header = rowValue.header.toStdString();
    row.kind = rowValue.kind == LineKind::Addition
                   ? DiffLineKind::Addition
                   : rowValue.kind == LineKind::Deletion ? DiffLineKind::Deletion : DiffLineKind::Context;
    row.oldLine = rowValue.oldLine;
    row.newLine = rowValue.newLine;
    const QByteArray textUtf8 = rowValue.text.toUtf8();
    row.textRange = prepared.textRope.append(std::string(textUtf8.constData(), textUtf8.size()));
    row.textWidth = metrics.horizontalAdvance(rowValue.text);
    row.tokens = convertTokens(rowValue.tokens);
    row.changeSpans = convertTokens(rowValue.changeSpans);
    prepared.maxTextWidth = std::max(prepared.maxTextWidth, row.textWidth);
    prepared.sourceRows.push_back(std::move(row));
  }

  return prepared;
}

}  // namespace diffy
