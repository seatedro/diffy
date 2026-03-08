#include "app/QtDiffTypes.h"

namespace diffy {

QString lineKindToQString(LineKind kind) {
  return QString::fromLatin1(lineKindToString(kind).data(),
                             static_cast<int>(lineKindToString(kind).size()));
}

QVariantMap lineToVariant(const DiffLine& line) {
  QVariantList tokenRanges;
  for (const TokenSpan& token : line.tokens) {
    tokenRanges.push_back(QVariantMap{{"start", token.start}, {"length", token.length}});
  }

  return QVariantMap{
      {"oldLine", line.oldLine},
      {"newLine", line.newLine},
      {"kind", lineKindToQString(line.kind)},
      {"text", QString::fromUtf8(line.text)},
      {"tokens", tokenRanges},
  };
}

QVariantMap hunkToVariant(const Hunk& hunk) {
  QVariantList lines;
  for (const DiffLine& line : hunk.lines) {
    lines.push_back(lineToVariant(line));
  }

  return QVariantMap{{"header", QString::fromUtf8(hunk.header)},
                     {"collapsed", hunk.collapsed},
                     {"lines", lines}};
}

QVariantMap fileDiffToVariant(const FileDiff& file) {
  QVariantList hunks;
  for (const Hunk& hunk : file.hunks) {
    hunks.push_back(hunkToVariant(hunk));
  }

  return QVariantMap{{"path", QString::fromUtf8(file.path)},
                     {"status", QString::fromUtf8(file.status)},
                     {"isBinary", file.isBinary},
                     {"additions", file.additions},
                     {"deletions", file.deletions},
                     {"hunks", hunks}};
}

QVariantMap fileDiffSummaryToVariant(const FileDiff& file) {
  return QVariantMap{{"path", QString::fromUtf8(file.path)},
                     {"status", QString::fromUtf8(file.status)},
                     {"isBinary", file.isBinary},
                     {"additions", file.additions},
                     {"deletions", file.deletions}};
}

QVariantList filesToVariantList(const std::vector<FileDiff>& files) {
  QVariantList result;
  result.reserve(static_cast<qsizetype>(files.size()));
  for (const FileDiff& file : files) {
    result.push_back(fileDiffSummaryToVariant(file));
  }
  return result;
}

}  // namespace diffy
