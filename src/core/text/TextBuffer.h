#pragma once

#include <string>
#include <string_view>

namespace diffy {

struct TextRange {
  size_t start = 0;
  size_t length = 0;

  bool isEmpty() const {
    return length == 0;
  }
};

class TextBuffer {
 public:
  TextBuffer() = default;

  void clear();
  void reserve(size_t capacity);
  TextRange append(std::string_view text);
  std::string_view view(const TextRange& range) const;
  std::string slice(const TextRange& range) const;
  size_t size() const;

 private:
  std::string buffer_;
};

}  // namespace diffy
