#pragma once

#include <QVariant>
#include <QString>
#include <QVector>

namespace diffy {

enum class LineKind {
  Context,
  Addition,
  Deletion,
};

struct TokenSpan {
  int start = 0;
  int length = 0;
};

struct DiffLine {
  int oldLine = -1;
  int newLine = -1;
  LineKind kind = LineKind::Context;
  QString text;
  QVector<TokenSpan> tokens;
};

struct Hunk {
  QString header;
  bool collapsed = false;
  QVector<DiffLine> lines;
};

struct FileDiff {
  QString path;
  QString status = "M";
  bool isBinary = false;
  int additions = 0;
  int deletions = 0;
  QVector<Hunk> hunks;
};

struct DiffDocument {
  QString leftRevision;
  QString rightRevision;
  QVector<FileDiff> files;
};

QString lineKindToString(LineKind kind);
QVariantMap lineToVariant(const DiffLine& line);
QVariantMap hunkToVariant(const Hunk& hunk);
QVariantMap fileDiffToVariant(const FileDiff& file);
QVariantList filesToVariantList(const QVector<FileDiff>& files);

}  // namespace diffy
