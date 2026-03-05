#include <QtTest/QtTest>

#include "core/CompareSpec.h"

using namespace diffy;

class CompareSpecTest : public QObject {
  Q_OBJECT

 private slots:
  void parsesModeStrings() {
    QCOMPARE(compareModeFromString("two-dot"), CompareMode::TwoDot);
    QCOMPARE(compareModeFromString("three-dot"), CompareMode::ThreeDot);
    QCOMPARE(compareModeFromString("single-commit"), CompareMode::SingleCommit);
    QCOMPARE(compareModeFromString("unknown"), CompareMode::TwoDot);
  }

  void roundtripsRendererAndLayout() {
    QCOMPARE(rendererKindToString(rendererKindFromString("builtin")), QString("builtin"));
    QCOMPARE(rendererKindToString(rendererKindFromString("difftastic")), QString("difftastic"));

    QCOMPARE(layoutModeToString(layoutModeFromString("split")), QString("split"));
    QCOMPARE(layoutModeToString(layoutModeFromString("anything")), QString("unified"));
  }
};

QTEST_MAIN(CompareSpecTest)
#include "test_compare_spec.moc"
