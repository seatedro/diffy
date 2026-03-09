#include "core/rendering/DiffLayoutEngine.h"

#include <algorithm>
#include <cmath>

namespace diffy {
namespace {

double wrappedLineHeight(double baseHeight, bool wrapEnabled, double textWidth, double availableWidth) {
  if (!wrapEnabled || baseHeight <= 0 || availableWidth <= 0 || textWidth <= availableWidth) {
    return baseHeight;
  }
  return baseHeight * std::ceil(textWidth / availableWidth);
}

}  // namespace

bool DiffLayoutEngine::sameConfig(const DiffLayoutConfig& lhs, const DiffLayoutConfig& rhs) {
  return lhs.mode == rhs.mode && lhs.wrapEnabled == rhs.wrapEnabled &&
         std::abs(lhs.rowHeight - rhs.rowHeight) <= 0.001 &&
         std::abs(lhs.hunkHeight - rhs.hunkHeight) <= 0.001 &&
         std::abs(lhs.fileHeaderHeight - rhs.fileHeaderHeight) <= 0.001 &&
         std::abs(lhs.unifiedWrapWidth - rhs.unifiedWrapWidth) <= 0.001 &&
         std::abs(lhs.splitWrapWidth - rhs.splitWrapWidth) <= 0.001;
}

DiffLayoutEngine::LayoutCacheEntry& DiffLayoutEngine::layoutCache(DiffLayoutMode mode) {
  return mode == DiffLayoutMode::Unified ? unifiedLayoutCache_ : splitLayoutCache_;
}

const DiffLayoutEngine::LayoutCacheEntry& DiffLayoutEngine::layoutCache(DiffLayoutMode mode) const {
  return mode == DiffLayoutMode::Unified ? unifiedLayoutCache_ : splitLayoutCache_;
}

void DiffLayoutEngine::invalidateLayoutCaches() {
  unifiedLayoutCache_ = {};
  splitLayoutCache_ = {};
}

void DiffLayoutEngine::recomputeLineNumberDigits() {
  int maxLineNumber = 0;
  if (fileHeaderRow_) {
    maxLineNumber = std::max(maxLineNumber, std::max(fileHeaderRow_->oldLine, fileHeaderRow_->newLine));
  }
  for (const DiffSourceRow& row : sourceRows_) {
    maxLineNumber = std::max(maxLineNumber, std::max(row.oldLine, row.newLine));
  }
  lineNumberDigits_ = std::max(3, static_cast<int>(std::to_string(std::max(0, maxLineNumber)).size()));
}

void DiffLayoutEngine::setFileHeader(std::optional<DiffSourceRow> row) {
  const bool hadHeader = fileHeaderRow_.has_value();
  fileHeaderRow_ = std::move(row);
  recomputeLineNumberDigits();
  if (hadHeader != fileHeaderRow_.has_value()) {
    clearTopologyCaches();
    return;
  }

  if (!fileHeaderRow_) {
    return;
  }

  auto updateHeaderRow = [this](std::vector<DiffDisplayRow>& rows) {
    if (rows.empty() || rows.front().rowType != DiffRowType::FileHeader) {
      return;
    }
    rows.front().header = fileHeaderRow_->header;
    rows.front().detail = fileHeaderRow_->detail;
  };
  updateHeaderRow(unifiedTopologyRows_);
  updateHeaderRow(splitTopologyRows_);
  updateHeaderRow(displayRows_);
  invalidateLayoutCaches();
}

void DiffLayoutEngine::setSourceRows(std::vector<DiffSourceRow> rows, TokenBuffer tokenBuffer) {
  sourceRows_ = std::move(rows);
  sourceTokenBuffer_ = std::move(tokenBuffer);
  recomputeLineNumberDigits();
  clearTopologyCaches();
}

void DiffLayoutEngine::clearTopologyCaches() {
  unifiedTopologyRows_.clear();
  unifiedTopologyTokenBuffer_.clear();
  splitTopologyRows_.clear();
  splitTopologyTokenBuffer_.clear();
  displayRows_.clear();
  displayTokenBuffer_.clear();
  rowOffsets_.clear();
  contentHeight_ = 0;
  invalidateLayoutCaches();
}

void DiffLayoutEngine::rebuildTopology(std::vector<DiffDisplayRow>& targetRows,
                                        TokenBuffer& targetTokenBuffer,
                                        DiffLayoutMode mode) const {
  targetRows.clear();
  targetTokenBuffer.clear();

  auto copyTokenRange = [&](TokenRange srcRange) -> TokenRange {
    if (srcRange.empty()) return {};
    return targetTokenBuffer.append(sourceTokenBuffer_.begin(srcRange), srcRange.count);
  };

  auto appendRow = [&](DiffDisplayRow row) {
    targetRows.push_back(std::move(row));
  };

  if (fileHeaderRow_) {
    DiffDisplayRow row;
    row.rowType = fileHeaderRow_->rowType;
    row.header = fileHeaderRow_->header;
    row.detail = fileHeaderRow_->detail;
    appendRow(std::move(row));
  }

  if (mode == DiffLayoutMode::Unified) {
    for (const DiffSourceRow& sourceRow : sourceRows_) {
      DiffDisplayRow row;
      row.rowType = sourceRow.rowType;
      row.header = sourceRow.header;
      row.detail = sourceRow.detail;
      row.textWidth = sourceRow.textWidth;
      row.kind = sourceRow.kind;
      row.oldLine = sourceRow.oldLine;
      row.newLine = sourceRow.newLine;
      row.tokens = copyTokenRange(sourceRow.tokens);
      row.changeSpans = copyTokenRange(sourceRow.changeSpans);
      row.textRange = sourceRow.textRange;
      appendRow(std::move(row));
    }
  } else {
    for (size_t index = 0; index < sourceRows_.size(); ++index) {
      const DiffSourceRow& sourceRow = sourceRows_.at(index);
      if (sourceRow.rowType == DiffRowType::FileHeader || sourceRow.rowType == DiffRowType::Hunk) {
        DiffDisplayRow row;
        row.rowType = sourceRow.rowType;
        row.header = sourceRow.header;
        row.detail = sourceRow.detail;
        appendRow(std::move(row));
        continue;
      }

      if (sourceRow.kind == DiffLineKind::Context) {
        DiffDisplayRow row;
        row.rowType = DiffRowType::Line;
        row.textWidth = sourceRow.textWidth;
        row.kind = DiffLineKind::Context;
        row.oldLine = sourceRow.oldLine;
        row.newLine = sourceRow.newLine;
        row.leftKind = DiffLineKind::Context;
        row.rightKind = DiffLineKind::Context;
        row.leftLine = sourceRow.oldLine;
        row.rightLine = sourceRow.newLine;
        row.leftTokens = copyTokenRange(sourceRow.tokens);
        row.rightTokens = copyTokenRange(sourceRow.tokens);
        row.leftTextRange = sourceRow.textRange;
        row.rightTextRange = sourceRow.textRange;
        appendRow(std::move(row));
        continue;
      }

      std::vector<size_t> deletionIndices;
      std::vector<size_t> additionIndices;

      while (index < sourceRows_.size()) {
        const DiffSourceRow& blockRow = sourceRows_.at(index);
        if (blockRow.rowType != DiffRowType::Line || blockRow.kind == DiffLineKind::Context) {
          --index;
          break;
        }

        if (blockRow.kind == DiffLineKind::Deletion) {
          deletionIndices.push_back(index);
        } else if (blockRow.kind == DiffLineKind::Addition) {
          additionIndices.push_back(index);
        }
        ++index;
      }

      const size_t rowCount = std::max(deletionIndices.size(), additionIndices.size());
      for (size_t rowIndex = 0; rowIndex < rowCount; ++rowIndex) {
        DiffDisplayRow row;
        row.rowType = DiffRowType::Line;
        row.kind = DiffLineKind::Change;

        if (rowIndex < deletionIndices.size()) {
          const DiffSourceRow& left = sourceRows_.at(deletionIndices[rowIndex]);
          row.leftKind = DiffLineKind::Deletion;
          row.leftLine = left.oldLine;
          row.leftTokens = copyTokenRange(left.tokens);
          row.leftChangeSpans = copyTokenRange(left.changeSpans);
          row.leftTextRange = left.textRange;
          row.oldLine = left.oldLine;
        }

        if (rowIndex < additionIndices.size()) {
          const DiffSourceRow& right = sourceRows_.at(additionIndices[rowIndex]);
          row.rightKind = DiffLineKind::Addition;
          row.rightLine = right.newLine;
          row.rightTokens = copyTokenRange(right.tokens);
          row.rightChangeSpans = copyTokenRange(right.changeSpans);
          row.rightTextRange = right.textRange;
          row.newLine = right.newLine;
        }

        const double leftTextWidth = rowIndex < deletionIndices.size() ? sourceRows_.at(deletionIndices[rowIndex]).textWidth : 0;
        const double rightTextWidth = rowIndex < additionIndices.size() ? sourceRows_.at(additionIndices[rowIndex]).textWidth : 0;
        row.textWidth = std::max(leftTextWidth, rightTextWidth);
        appendRow(std::move(row));
      }
    }
  }
}

void DiffLayoutEngine::ensureLayoutCache(const DiffLayoutConfig& config) {
  LayoutCacheEntry& entry = layoutCache(config.mode);
  if (entry.valid && sameConfig(entry.config, config)) {
    return;
  }

  std::vector<DiffDisplayRow>* topologyRows = nullptr;
  TokenBuffer* topologyTokenBuffer = nullptr;
  if (config.mode == DiffLayoutMode::Unified) {
    if (unifiedTopologyRows_.empty() && (!sourceRows_.empty() || fileHeaderRow_)) {
      rebuildTopology(unifiedTopologyRows_, unifiedTopologyTokenBuffer_, DiffLayoutMode::Unified);
    }
    topologyRows = &unifiedTopologyRows_;
    topologyTokenBuffer = &unifiedTopologyTokenBuffer_;
  } else {
    if (splitTopologyRows_.empty() && (!sourceRows_.empty() || fileHeaderRow_)) {
      rebuildTopology(splitTopologyRows_, splitTopologyTokenBuffer_, DiffLayoutMode::Split);
    }
    topologyRows = &splitTopologyRows_;
    topologyTokenBuffer = &splitTopologyTokenBuffer_;
  }

  entry.rows = *topologyRows;
  entry.tokenBuffer = *topologyTokenBuffer;
  entry.rowOffsets.clear();
  entry.rowOffsets.reserve(entry.rows.size());

  double top = 0;
  for (DiffDisplayRow& row : entry.rows) {
    row.top = top;
    if (row.rowType == DiffRowType::FileHeader) {
      row.height = config.fileHeaderHeight;
    } else if (row.rowType == DiffRowType::Hunk) {
      row.height = config.hunkHeight;
    } else {
      const double wrapWidth =
          config.mode == DiffLayoutMode::Split ? config.splitWrapWidth : config.unifiedWrapWidth;
      row.height = wrappedLineHeight(config.rowHeight, config.wrapEnabled, row.textWidth, wrapWidth);
    }
    entry.rowOffsets.push_back(top);
    top += row.height;
  }

  entry.contentHeight = top;
  entry.config = config;
  entry.valid = true;
}

void DiffLayoutEngine::prewarm(const DiffLayoutConfig& config) {
  ensureLayoutCache(config);
}

const std::vector<DiffDisplayRow>& DiffLayoutEngine::cachedRows(const DiffLayoutConfig& config) {
  ensureLayoutCache(config);
  return layoutCache(config.mode).rows;
}

const TokenBuffer& DiffLayoutEngine::cachedTokenBuffer(const DiffLayoutConfig& config) {
  ensureLayoutCache(config);
  return layoutCache(config.mode).tokenBuffer;
}

int DiffLayoutEngine::rowIndexAtY(const DiffLayoutConfig& config, double y) {
  ensureLayoutCache(config);
  const LayoutCacheEntry& entry = layoutCache(config.mode);
  if (entry.rows.empty()) {
    return -1;
  }

  const auto it = std::upper_bound(entry.rowOffsets.cbegin(), entry.rowOffsets.cend(), y);
  if (it == entry.rowOffsets.cbegin()) {
    return 0;
  }
  return std::clamp(static_cast<int>(std::distance(entry.rowOffsets.cbegin(), it) - 1), 0,
                    static_cast<int>(entry.rows.size() - 1));
}

int DiffLayoutEngine::stickyHunkRowIndexAtY(const DiffLayoutConfig& config, double y) {
  ensureLayoutCache(config);
  const LayoutCacheEntry& entry = layoutCache(config.mode);
  int stickyIndex = -1;
  for (int rowIndex = 0; rowIndex < static_cast<int>(entry.rows.size()); ++rowIndex) {
    const DiffDisplayRow& row = entry.rows.at(rowIndex);
    if (row.rowType == DiffRowType::Hunk && row.top <= y) {
      stickyIndex = rowIndex;
    }
    if (row.top > y) {
      break;
    }
  }
  return stickyIndex;
}

int DiffLayoutEngine::fileHeaderRowIndex(const DiffLayoutConfig& config) {
  ensureLayoutCache(config);
  const LayoutCacheEntry& entry = layoutCache(config.mode);
  for (int rowIndex = 0; rowIndex < static_cast<int>(entry.rows.size()); ++rowIndex) {
    if (entry.rows.at(rowIndex).rowType == DiffRowType::FileHeader) {
      return rowIndex;
    }
  }
  return -1;
}

void DiffLayoutEngine::rebuild(const DiffLayoutConfig& config) {
  ensureLayoutCache(config);
  const LayoutCacheEntry& entry = layoutCache(config.mode);
  displayRows_ = entry.rows;
  displayTokenBuffer_ = entry.tokenBuffer;
  rowOffsets_ = entry.rowOffsets;
  contentHeight_ = entry.contentHeight;
}

const std::vector<DiffDisplayRow>& DiffLayoutEngine::rows() const {
  return displayRows_;
}

const TokenBuffer& DiffLayoutEngine::tokenBuffer() const {
  return displayTokenBuffer_;
}

double DiffLayoutEngine::contentHeight() const {
  return contentHeight_;
}

int DiffLayoutEngine::lineNumberDigits() const {
  return lineNumberDigits_;
}

int DiffLayoutEngine::rowIndexAtY(double y) const {
  if (displayRows_.empty()) {
    return -1;
  }

  const auto it = std::upper_bound(rowOffsets_.cbegin(), rowOffsets_.cend(), y);
  if (it == rowOffsets_.cbegin()) {
    return 0;
  }
  return std::clamp(static_cast<int>(std::distance(rowOffsets_.cbegin(), it) - 1), 0,
                    static_cast<int>(displayRows_.size() - 1));
}

int DiffLayoutEngine::fileHeaderRowIndex() const {
  for (int rowIndex = 0; rowIndex < static_cast<int>(displayRows_.size()); ++rowIndex) {
    if (displayRows_.at(rowIndex).rowType == DiffRowType::FileHeader) {
      return rowIndex;
    }
  }
  return -1;
}

int DiffLayoutEngine::stickyHunkRowIndexAtY(double y) const {
  int stickyIndex = -1;
  for (int rowIndex = 0; rowIndex < static_cast<int>(displayRows_.size()); ++rowIndex) {
    const DiffDisplayRow& row = displayRows_.at(rowIndex);
    if (row.rowType == DiffRowType::Hunk && row.top <= y) {
      stickyIndex = rowIndex;
    }
    if (row.top > y) {
      break;
    }
  }
  return stickyIndex;
}

int DiffLayoutEngine::nextHunkRowIndex(int rowIndex) const {
  const int start = std::max(0, rowIndex + 1);
  for (int index = start; index < static_cast<int>(displayRows_.size()); ++index) {
    if (displayRows_.at(index).rowType == DiffRowType::Hunk) {
      return index;
    }
  }
  return -1;
}

int DiffLayoutEngine::previousHunkRowIndex(int rowIndex) const {
  const int start = std::min(rowIndex - 1, static_cast<int>(displayRows_.size()) - 1);
  for (int index = start; index >= 0; --index) {
    if (displayRows_.at(index).rowType == DiffRowType::Hunk) {
      return index;
    }
  }
  return -1;
}

}  // namespace diffy
