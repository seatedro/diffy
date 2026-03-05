#include "core/DiffTypes.h"

namespace diffy {

QString lineKindToString(LineKind kind) {
  switch (kind) {
    case LineKind::Addition:
      return "add";
    case LineKind::Deletion:
      return "del";
    case LineKind::Context:
      return "ctx";
  }
  return "ctx";
}

QVariantMap lineToVariant(const DiffLine& line) {
  QVariantList tokenRanges;
  for (const TokenSpan& token : line.tokens) {
    tokenRanges.push_back(QVariantMap{{"start", token.start}, {"length", token.length}});
  }

  return QVariantMap{
      {"oldLine", line.oldLine},
      {"newLine", line.newLine},
      {"kind", lineKindToString(line.kind)},
      {"text", line.text},
      {"tokens", tokenRanges},
  };
}

QVariantMap hunkToVariant(const Hunk& hunk) {
  QVariantList lines;
  for (const DiffLine& line : hunk.lines) {
    lines.push_back(lineToVariant(line));
  }

  return QVariantMap{{"header", hunk.header}, {"collapsed", hunk.collapsed}, {"lines", lines}};
}

QVariantMap fileDiffToVariant(const FileDiff& file) {
  QVariantList hunks;
  for (const Hunk& hunk : file.hunks) {
    hunks.push_back(hunkToVariant(hunk));
  }

  return QVariantMap{{"path", file.path},
                     {"status", file.status},
                     {"isBinary", file.isBinary},
                     {"additions", file.additions},
                     {"deletions", file.deletions},
                     {"hunks", hunks}};
}

QVariantList filesToVariantList(const QVector<FileDiff>& files) {
  QVariantList result;
  for (const FileDiff& file : files) {
    result.push_back(fileDiffToVariant(file));
  }
  return result;
}

}  // namespace diffy
