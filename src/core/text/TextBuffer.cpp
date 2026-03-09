#include "core/text/TextBuffer.h"

namespace diffy {

void TextBuffer::clear() {
  buffer_.clear();
}

void TextBuffer::reserve(size_t capacity) {
  buffer_.reserve(capacity);
}

TextRange TextBuffer::append(std::string_view text) {
  const TextRange range{buffer_.size(), text.size()};
  buffer_.append(text);
  return range;
}

std::string_view TextBuffer::view(const TextRange& range) const {
  if (range.length == 0 || range.start >= buffer_.size()) {
    return {};
  }
  return std::string_view(buffer_.data() + range.start, range.length);
}

std::string TextBuffer::slice(const TextRange& range) const {
  if (range.length == 0 || range.start >= buffer_.size()) {
    return {};
  }
  return buffer_.substr(range.start, range.length);
}

size_t TextBuffer::size() const {
  return buffer_.size();
}

}  // namespace diffy
