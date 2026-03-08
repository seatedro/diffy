#pragma once

#include <string>
#include <string_view>
#include <vector>

namespace diffy {

struct TextRange {
  size_t start = 0;
  size_t length = 0;

  bool isEmpty() const {
    return length == 0;
  }
};

class TextRope {
 public:
  TextRope() = default;

  void clear();
  TextRange append(std::string_view text);
  std::string slice(const TextRange& range) const;
  size_t size() const;

 private:
  struct Chunk {
    size_t start = 0;
    std::string text;
  };

  std::vector<Chunk> chunks_;
  size_t size_ = 0;
};

}  // namespace diffy
