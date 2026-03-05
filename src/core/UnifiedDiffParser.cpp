#include "core/UnifiedDiffParser.h"

#include <QRegularExpression>

namespace diffy {
namespace {

QString parsePathFromDiffHeader(const QString& line) {
  const QStringList parts = line.split(' ', Qt::SkipEmptyParts);
  if (parts.size() >= 4) {
    QString rhs = parts.at(3);
    if (rhs.startsWith("b/")) {
      rhs = rhs.mid(2);
    }
    return rhs;
  }
  return "unknown";
}

QVector<TokenSpan> fullLineTokens(const QString& text) {
  if (text.isEmpty()) {
    return {};
  }
  return QVector<TokenSpan>{TokenSpan{0, static_cast<int>(text.size())}};
}

}  // namespace

DiffDocument UnifiedDiffParser::parse(const QString& leftRevision,
                                      const QString& rightRevision,
                                      const QString& diffText) const {
  DiffDocument doc;
  doc.leftRevision = leftRevision;
  doc.rightRevision = rightRevision;

  FileDiff* currentFile = nullptr;
  Hunk* currentHunk = nullptr;
  int oldLine = 0;
  int newLine = 0;

  const QRegularExpression hunkRegex(R"(^@@\s+-(\d+)(?:,\d+)?\s+\+(\d+)(?:,\d+)?\s+@@)");

  const QStringList lines = diffText.split('\n', Qt::KeepEmptyParts);
  for (qsizetype i = 0; i < lines.size(); ++i) {
    const QString& line = lines.at(i);
    if (i == lines.size() - 1 && line.isEmpty()) {
      continue;
    }

    if (line.startsWith("diff --git ")) {
      FileDiff file;
      file.path = parsePathFromDiffHeader(line);
      file.status = "M";
      doc.files.push_back(file);
      currentFile = &doc.files.last();
      currentHunk = nullptr;
      continue;
    }

    if (currentFile == nullptr) {
      continue;
    }

    if (line.startsWith("new file mode")) {
      currentFile->status = "A";
      continue;
    }
    if (line.startsWith("deleted file mode")) {
      currentFile->status = "D";
      continue;
    }
    if (line.startsWith("rename from") || line.startsWith("rename to")) {
      currentFile->status = "R";
      continue;
    }
    if (line.startsWith("Binary files ")) {
      currentFile->isBinary = true;
      continue;
    }

    if (line.startsWith("@@ ")) {
      Hunk hunk;
      hunk.header = line;
      hunk.collapsed = false;
      currentFile->hunks.push_back(hunk);
      currentHunk = &currentFile->hunks.last();

      const QRegularExpressionMatch match = hunkRegex.match(line);
      if (match.hasMatch()) {
        oldLine = match.captured(1).toInt();
        newLine = match.captured(2).toInt();
      }
      continue;
    }

    if (currentHunk == nullptr) {
      continue;
    }

    if (line.startsWith("+++") || line.startsWith("---") || line.startsWith("\\ No newline")) {
      continue;
    }

    if (line.startsWith('+')) {
      const QString text = line.mid(1);
      currentHunk->lines.push_back(DiffLine{-1, newLine++, LineKind::Addition, text, fullLineTokens(text)});
      currentFile->additions += 1;
      continue;
    }

    if (line.startsWith('-')) {
      const QString text = line.mid(1);
      currentHunk->lines.push_back(DiffLine{oldLine++, -1, LineKind::Deletion, text, fullLineTokens(text)});
      currentFile->deletions += 1;
      continue;
    }

    const QString contextText = line.startsWith(' ') ? line.mid(1) : line;
    currentHunk->lines.push_back(DiffLine{oldLine++, newLine++, LineKind::Context, contextText, {}});
  }

  return doc;
}

}  // namespace diffy
