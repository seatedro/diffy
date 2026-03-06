#pragma once

#include <string>
#include <string_view>
#include <vector>

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
  std::string text;
  std::vector<TokenSpan> tokens;
};

struct Hunk {
  std::string header;
  bool collapsed = false;
  std::vector<DiffLine> lines;
};

struct FileDiff {
  std::string path;
  std::string status = "M";
  bool isBinary = false;
  int additions = 0;
  int deletions = 0;
  std::vector<Hunk> hunks;
};

struct DiffDocument {
  std::string leftRevision;
  std::string rightRevision;
  std::vector<FileDiff> files;
};

std::string_view lineKindToString(LineKind kind);
LineKind lineKindFromString(std::string_view value);

}  // namespace diffy
