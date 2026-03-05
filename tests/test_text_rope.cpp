#include <QtTest/QtTest>

#include "text/TextRope.h"

using namespace diffy;

class TextRopeTest : public QObject {
  Q_OBJECT

 private slots:
  void appendAndSliceAcrossChunks() {
    TextRope rope;
    const TextRange first = rope.append("hello");
    const TextRange second = rope.append(" world");
    const TextRange third = rope.append("!");

    QCOMPARE(rope.size(), 12);
    QCOMPARE(rope.slice(first), QString("hello"));
    QCOMPARE(rope.slice(second), QString(" world"));
    QCOMPARE(rope.slice(third), QString("!"));
    QCOMPARE(rope.slice(TextRange{3, 7}), QString("lo worl"));
  }

  void emptyAppendsDoNotBreakOffsets() {
    TextRope rope;
    const TextRange empty = rope.append("");
    const TextRange text = rope.append("abc");

    QCOMPARE(empty.length, 0);
    QCOMPARE(text.start, 0);
    QCOMPARE(text.length, 3);
    QCOMPARE(rope.slice(text), QString("abc"));
  }
};

QTEST_MAIN(TextRopeTest)
#include "test_text_rope.moc"
