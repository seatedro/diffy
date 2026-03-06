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
    QCOMPARE(rendererKindToString(rendererKindFromString("builtin")), std::string_view("builtin"));
    QCOMPARE(rendererKindToString(rendererKindFromString("difftastic")), std::string_view("difftastic"));

    QCOMPARE(layoutModeToString(layoutModeFromString("split")), std::string_view("split"));
    QCOMPARE(layoutModeToString(layoutModeFromString("anything")), std::string_view("unified"));
  }
};

QTEST_MAIN(CompareSpecTest)
#include "test_compare_spec.moc"
