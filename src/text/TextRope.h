#pragma once

#include <QVector>

#include <string>
#include <string_view>

namespace diffy {

struct TextRange {
  qsizetype start = 0;
  qsizetype length = 0;

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
  qsizetype size() const;

 private:
  struct Chunk {
    qsizetype start = 0;
    std::string text;
  };

  QVector<Chunk> chunks_;
  qsizetype size_ = 0;
};

}  // namespace diffy
