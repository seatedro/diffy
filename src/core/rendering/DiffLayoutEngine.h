#pragma once

#include <optional>
#include <string>
#include <vector>

#include "core/syntax/SyntaxTypes.h"
#include "core/text/TextBuffer.h"

namespace diffy {

enum class DiffRowType {
  FileHeader,
  Hunk,
  Line,
};

enum class DiffLineKind {
  Context,
  Addition,
  Deletion,
  Spacer,
  Change,
};

enum class DiffLayoutMode {
  Unified,
  Split,
};

struct DiffTokenSpan {
  int start = 0;
  int length = 0;
  SyntaxTokenKind syntaxKind = SyntaxTokenKind::None;
};

struct TokenRange {
  uint32_t start = 0;
  uint32_t count = 0;

  bool empty() const { return count == 0; }
};

class TokenBuffer {
 public:
  void clear() { spans_.clear(); }

  void reserve(size_t capacity) { spans_.reserve(capacity); }

  TokenRange append(const DiffTokenSpan* data, size_t count) {
    TokenRange range{static_cast<uint32_t>(spans_.size()), static_cast<uint32_t>(count)};
    spans_.insert(spans_.end(), data, data + count);
    return range;
  }

  TokenRange append(const std::vector<DiffTokenSpan>& tokens) {
    return append(tokens.data(), tokens.size());
  }

  const DiffTokenSpan* data() const { return spans_.data(); }
  const DiffTokenSpan* begin(const TokenRange& range) const { return spans_.data() + range.start; }
  const DiffTokenSpan* end(const TokenRange& range) const { return spans_.data() + range.start + range.count; }
  size_t size() const { return spans_.size(); }

 private:
  std::vector<DiffTokenSpan> spans_;
};

struct DiffSourceRow {
  DiffRowType rowType = DiffRowType::Line;
  std::string header;
  std::string detail;
  DiffLineKind kind = DiffLineKind::Context;
  int oldLine = -1;
  int newLine = -1;
  double textWidth = 0;
  TokenRange tokens;
  TokenRange changeSpans;
  TextRange textRange;
};

struct DiffLayoutConfig {
  DiffLayoutMode mode = DiffLayoutMode::Unified;
  double rowHeight = 0;
  double hunkHeight = 0;
  double fileHeaderHeight = 0;
  bool wrapEnabled = false;
  double unifiedWrapWidth = 0;
  double splitWrapWidth = 0;
};

struct DiffDisplayRow {
  DiffRowType rowType = DiffRowType::Line;
  std::string header;
  std::string detail;
  double textWidth = 0;
  DiffLineKind kind = DiffLineKind::Context;
  int oldLine = -1;
  int newLine = -1;
  TokenRange tokens;
  TokenRange changeSpans;
  DiffLineKind leftKind = DiffLineKind::Spacer;
  DiffLineKind rightKind = DiffLineKind::Spacer;
  int leftLine = -1;
  int rightLine = -1;
  TokenRange leftTokens;
  TokenRange leftChangeSpans;
  TokenRange rightTokens;
  TokenRange rightChangeSpans;
  TextRange textRange;
  TextRange leftTextRange;
  TextRange rightTextRange;
  double top = 0;
  double height = 0;
};

class DiffLayoutEngine {
 public:
  void setFileHeader(std::optional<DiffSourceRow> row);
  void setSourceRows(std::vector<DiffSourceRow> rows, TokenBuffer tokenBuffer);
  void prewarm(const DiffLayoutConfig& config);
  void rebuild(const DiffLayoutConfig& config);
  const std::vector<DiffDisplayRow>& cachedRows(const DiffLayoutConfig& config);
  int rowIndexAtY(const DiffLayoutConfig& config, double y);
  int stickyHunkRowIndexAtY(const DiffLayoutConfig& config, double y);
  int fileHeaderRowIndex(const DiffLayoutConfig& config);

  const std::vector<DiffDisplayRow>& rows() const;
  const TokenBuffer& tokenBuffer() const;
  double contentHeight() const;
  int lineNumberDigits() const;

  int rowIndexAtY(double y) const;
  int fileHeaderRowIndex() const;
  int stickyHunkRowIndexAtY(double y) const;
  int nextHunkRowIndex(int rowIndex) const;
  int previousHunkRowIndex(int rowIndex) const;

 private:
  struct LayoutCacheEntry {
    bool valid = false;
    DiffLayoutConfig config;
    std::vector<DiffDisplayRow> rows;
    TokenBuffer tokenBuffer;
    std::vector<double> rowOffsets;
    double contentHeight = 0;
  };

  void recomputeLineNumberDigits();
  void clearTopologyCaches();
  void rebuildTopology(std::vector<DiffDisplayRow>& targetRows, TokenBuffer& targetTokenBuffer, DiffLayoutMode mode) const;
  void invalidateLayoutCaches();
  LayoutCacheEntry& layoutCache(DiffLayoutMode mode);
  const LayoutCacheEntry& layoutCache(DiffLayoutMode mode) const;
  void ensureLayoutCache(const DiffLayoutConfig& config);
  static bool sameConfig(const DiffLayoutConfig& lhs, const DiffLayoutConfig& rhs);

  std::optional<DiffSourceRow> fileHeaderRow_;
  std::vector<DiffSourceRow> sourceRows_;
  TokenBuffer sourceTokenBuffer_;
  std::vector<DiffDisplayRow> unifiedTopologyRows_;
  TokenBuffer unifiedTopologyTokenBuffer_;
  std::vector<DiffDisplayRow> splitTopologyRows_;
  TokenBuffer splitTopologyTokenBuffer_;
  LayoutCacheEntry unifiedLayoutCache_;
  LayoutCacheEntry splitLayoutCache_;
  std::vector<DiffDisplayRow> displayRows_;
  TokenBuffer displayTokenBuffer_;
  std::vector<double> rowOffsets_;
  double contentHeight_ = 0;
  int lineNumberDigits_ = 3;
};

}  // namespace diffy
