#include <QTest>

#include "core/diff/WordDiff.h"

class TestWordDiff : public QObject {
  Q_OBJECT

 private slots:
  void identicalLines() {
    auto result = diffy::computeWordDiff("hello world", "hello world");
    QVERIFY(result.leftTokens.empty());
    QVERIFY(result.rightTokens.empty());
  }

  void singleWordChange() {
    auto result = diffy::computeWordDiff("int foo = 1;", "int bar = 1;");
    QCOMPARE(result.leftTokens.size(), 1u);
    QCOMPARE(result.rightTokens.size(), 1u);
    QCOMPARE(result.leftTokens[0].start, 4);
    QCOMPARE(result.leftTokens[0].length, 3);
    QCOMPARE(result.rightTokens[0].start, 4);
    QCOMPARE(result.rightTokens[0].length, 3);
  }

  void emptyToContent() {
    auto result = diffy::computeWordDiff("", "hello world");
    QVERIFY(result.leftTokens.empty());
    QVERIFY(result.rightTokens.size() >= 1u);
  }

  void contentToEmpty() {
    auto result = diffy::computeWordDiff("hello world", "");
    QVERIFY(result.leftTokens.size() >= 1u);
    QVERIFY(result.rightTokens.empty());
  }

  void multipleChanges() {
    auto result = diffy::computeWordDiff("const int x = 10;", "let float y = 20;");
    QVERIFY(result.leftTokens.size() >= 2u);
    QVERIFY(result.rightTokens.size() >= 2u);
  }
};

QTEST_MAIN(TestWordDiff)
#include "test_word_diff.moc"
