#include "core/diff/UnifiedDiffParser.h"

#include <charconv>
#include <string>

namespace diffy {
namespace {

std::string_view trimTrailingCarriageReturn(std::string_view line) {
  if (!line.empty() && line.back() == '\r') {
    line.remove_suffix(1);
  }
  return line;
}

std::string_view nextToken(std::string_view* value) {
  while (!value->empty() && value->front() == ' ') {
    value->remove_prefix(1);
  }
  const size_t nextSpace = value->find(' ');
  const std::string_view token = value->substr(0, nextSpace);
  if (nextSpace == std::string_view::npos) {
    value->remove_prefix(value->size());
  } else {
    value->remove_prefix(nextSpace + 1);
  }
  return token;
}

std::string parsePathFromDiffHeader(std::string_view line) {
  std::string_view remainder = line;
  nextToken(&remainder);
  nextToken(&remainder);
  nextToken(&remainder);
  std::string_view rhs = nextToken(&remainder);
  if (rhs.starts_with("b/")) {
    rhs.remove_prefix(2);
  }
  return rhs.empty() ? std::string("unknown") : std::string(rhs);
}

std::vector<TokenSpan> fullLineTokens(const std::string& text) {
  if (text.empty()) {
    return {};
  }
  return {TokenSpan{0, static_cast<int>(text.size())}};
}

bool parsePositiveInt(std::string_view value, int* out) {
  int parsed = 0;
  const auto result = std::from_chars(value.data(), value.data() + value.size(), parsed);
  if (result.ec != std::errc() || result.ptr != value.data() + value.size() || parsed < 0) {
    return false;
  }
  if (out != nullptr) {
    *out = parsed;
  }
  return true;
}

bool parseHunkHeader(std::string_view line, int* oldLine, int* newLine) {
  const size_t minus = line.find('-');
  const size_t plus = line.find('+', minus == std::string_view::npos ? 0 : minus);
  if (minus == std::string_view::npos || plus == std::string_view::npos) {
    return false;
  }

  size_t oldEnd = minus + 1;
  while (oldEnd < line.size() && std::isdigit(static_cast<unsigned char>(line[oldEnd])) != 0) {
    ++oldEnd;
  }
  size_t newEnd = plus + 1;
  while (newEnd < line.size() && std::isdigit(static_cast<unsigned char>(line[newEnd])) != 0) {
    ++newEnd;
  }

  if (oldEnd == minus + 1 || newEnd == plus + 1) {
    return false;
  }

  return parsePositiveInt(line.substr(minus + 1, oldEnd - minus - 1), oldLine) &&
         parsePositiveInt(line.substr(plus + 1, newEnd - plus - 1), newLine);
}

}  // namespace

DiffDocument UnifiedDiffParser::parse(std::string_view leftRevision,
                                      std::string_view rightRevision,
                                      std::string_view diffText) const {
  DiffDocument doc;
  doc.leftRevision = std::string(leftRevision);
  doc.rightRevision = std::string(rightRevision);

  FileDiff* currentFile = nullptr;
  Hunk* currentHunk = nullptr;
  int oldLine = 0;
  int newLine = 0;

  size_t cursor = 0;
  while (cursor <= diffText.size()) {
    size_t nextBreak = diffText.find('\n', cursor);
    if (nextBreak == std::string_view::npos) {
      nextBreak = diffText.size();
    }

    std::string_view line = trimTrailingCarriageReturn(diffText.substr(cursor, nextBreak - cursor));
    cursor = nextBreak == diffText.size() ? diffText.size() + 1 : nextBreak + 1;

    if (line.empty() && nextBreak == diffText.size()) {
      continue;
    }

    if (line.starts_with("diff --git ")) {
      FileDiff file;
      file.path = parsePathFromDiffHeader(line);
      file.status = "M";
      doc.files.push_back(std::move(file));
      currentFile = &doc.files.back();
      currentHunk = nullptr;
      continue;
    }

    if (currentFile == nullptr) {
      continue;
    }

    if (line.starts_with("new file mode")) {
      currentFile->status = "A";
      continue;
    }
    if (line.starts_with("deleted file mode")) {
      currentFile->status = "D";
      continue;
    }
    if (line.starts_with("rename from") || line.starts_with("rename to")) {
      currentFile->status = "R";
      continue;
    }
    if (line.starts_with("Binary files ")) {
      currentFile->isBinary = true;
      continue;
    }

    if (line.starts_with("@@ ")) {
      Hunk hunk;
      hunk.header = std::string(line);
      hunk.collapsed = false;
      currentFile->hunks.push_back(std::move(hunk));
      currentHunk = &currentFile->hunks.back();
      parseHunkHeader(line, &oldLine, &newLine);
      continue;
    }

    if (currentHunk == nullptr) {
      continue;
    }

    if (line.starts_with("+++") || line.starts_with("---") || line.starts_with("\\ No newline")) {
      continue;
    }

    if (line.starts_with('+')) {
      std::string text(line.substr(1));
      currentHunk->lines.push_back(DiffLine{-1, newLine++, LineKind::Addition, text, fullLineTokens(text)});
      currentFile->additions += 1;
      continue;
    }

    if (line.starts_with('-')) {
      std::string text(line.substr(1));
      currentHunk->lines.push_back(DiffLine{oldLine++, -1, LineKind::Deletion, text, fullLineTokens(text)});
      currentFile->deletions += 1;
      continue;
    }

    std::string contextText(line.starts_with(' ') ? line.substr(1) : line);
    currentHunk->lines.push_back(DiffLine{oldLine++, newLine++, LineKind::Context, std::move(contextText), {}});
  }

  return doc;
}

}  // namespace diffy
