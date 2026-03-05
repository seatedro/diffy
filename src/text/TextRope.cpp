#include "text/TextRope.h"

#include <algorithm>

namespace diffy {

void TextRope::clear() {
  chunks_.clear();
  size_ = 0;
}

TextRange TextRope::append(const QString& text) {
  const TextRange range{size_, text.size()};
  if (!text.isEmpty()) {
    chunks_.push_back(Chunk{size_, text});
    size_ += text.size();
  }
  return range;
}

QString TextRope::slice(const TextRange& range) const {
  if (range.length <= 0 || chunks_.isEmpty()) {
    return {};
  }

  const qsizetype end = range.start + range.length;
  QString result;
  result.reserve(range.length);

  for (const Chunk& chunk : chunks_) {
    const qsizetype chunkStart = chunk.start;
    const qsizetype chunkEnd = chunk.start + chunk.text.size();
    if (chunkEnd <= range.start) {
      continue;
    }
    if (chunkStart >= end) {
      break;
    }

    const qsizetype sliceStart = std::max(range.start, chunkStart);
    const qsizetype sliceEnd = std::min(end, chunkEnd);
    const qsizetype localStart = sliceStart - chunkStart;
    const qsizetype localLength = sliceEnd - sliceStart;
    result.append(chunk.text.mid(localStart, localLength));
  }

  return result;
}

qsizetype TextRope::size() const {
  return size_;
}

}  // namespace diffy
