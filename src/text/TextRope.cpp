#include "text/TextRope.h"

#include <algorithm>

namespace diffy {

void TextRope::clear() {
  chunks_.clear();
  size_ = 0;
}

TextRange TextRope::append(std::string_view text) {
  const TextRange range{size_, static_cast<qsizetype>(text.size())};
  if (!text.empty()) {
    chunks_.push_back(Chunk{size_, std::string(text)});
    size_ += text.size();
  }
  return range;
}

std::string TextRope::slice(const TextRange& range) const {
  if (range.length <= 0 || chunks_.isEmpty()) {
    return {};
  }

  const qsizetype end = range.start + range.length;
  std::string result;
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
    result.append(chunk.text.substr(localStart, localLength));
  }

  return result;
}

qsizetype TextRope::size() const {
  return size_;
}

}  // namespace diffy
