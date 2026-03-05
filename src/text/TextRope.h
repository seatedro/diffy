#pragma once

#include <QString>
#include <QVector>

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
  TextRange append(const QString& text);
  QString slice(const TextRange& range) const;
  qsizetype size() const;

 private:
  struct Chunk {
    qsizetype start = 0;
    QString text;
  };

  QVector<Chunk> chunks_;
  qsizetype size_ = 0;
};

}  // namespace diffy
