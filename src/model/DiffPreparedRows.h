#pragma once

#include <QString>

#include <vector>

#include "model/DiffDisplayModel.h"
#include "text/TextRope.h"

namespace diffy {

struct FlattenedDiffRow;

struct PreparedRowsCacheKey {
  int compareGeneration = 0;
  QString filePath;
  QString family;

  bool operator==(const PreparedRowsCacheKey& other) const = default;
};

inline size_t qHash(const PreparedRowsCacheKey& key, size_t seed = 0) {
  return qHashMulti(seed, key.compareGeneration, key.filePath, key.family);
}

struct PreparedRows {
  TextRope textRope;
  std::vector<DiffSourceRow> sourceRows;
  double maxTextWidth = 0.0;
};

PreparedRows prepareRowsForSurface(const std::vector<FlattenedDiffRow>& rows, const QString& monoFontFamily);

}  // namespace diffy
