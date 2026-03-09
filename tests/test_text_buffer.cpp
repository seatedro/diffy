#include <QtTest/QtTest>

#include "core/text/TextBuffer.h"

using namespace diffy;

class TextBufferTest : public QObject {
  Q_OBJECT

 private slots:
  void appendAndSliceAcrossChunks() {
    TextBuffer buf;
    const TextRange first = buf.append("hello");
    const TextRange second = buf.append(" world");
    const TextRange third = buf.append("!");

    QCOMPARE(buf.size(), 12);
    QCOMPARE(QString::fromStdString(buf.slice(first)), QString("hello"));
    QCOMPARE(QString::fromStdString(buf.slice(second)), QString(" world"));
    QCOMPARE(QString::fromStdString(buf.slice(third)), QString("!"));
    QCOMPARE(QString::fromStdString(buf.slice(TextRange{3, 7})), QString("lo worl"));
  }

  void emptyAppendsDoNotBreakOffsets() {
    TextBuffer buf;
    const TextRange empty = buf.append("");
    const TextRange text = buf.append("abc");

    QCOMPARE(empty.length, 0);
    QCOMPARE(text.start, 0);
    QCOMPARE(text.length, 3);
    QCOMPARE(QString::fromStdString(buf.slice(text)), QString("abc"));
  }
};

QTEST_MAIN(TextBufferTest)
#include "test_text_buffer.moc"
