#pragma once

#include <functional>
#include <string>
#include <vector>

#include "core/rendering/DiffLayoutEngine.h"
#include "core/rendering/FlatDiffRows.h"
#include "core/text/TextBuffer.h"

namespace diffy {

struct PreparedRowsCacheKey {
  int compareGeneration = 0;
  std::string filePath;
  std::string family;

  bool operator==(const PreparedRowsCacheKey& other) const = default;
};

inline size_t qHash(const PreparedRowsCacheKey& key, size_t seed = 0) {
  auto combine = [](size_t lhs, size_t rhs) {
    return lhs ^ (rhs + 0x9e3779b97f4a7c15ULL + (lhs << 6) + (lhs >> 2));
  };
  size_t hash = seed;
  hash = combine(hash, std::hash<int>{}(key.compareGeneration));
  hash = combine(hash, std::hash<std::string>{}(key.filePath));
  hash = combine(hash, std::hash<std::string>{}(key.family));
  return hash;
}

struct PreparedRows {
  TextBuffer textBuffer;
  TokenBuffer tokenBuffer;
  std::vector<DiffSourceRow> sourceRows;
  double maxTextWidth = 0.0;
};

using TextWidthMeasure = std::function<double(std::string_view)>;

PreparedRows prepareRowsForDisplay(const std::vector<FlatDiffRow>& rows, const TextWidthMeasure& measureTextWidth);

}  // namespace diffy
