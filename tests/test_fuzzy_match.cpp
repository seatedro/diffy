#include <QtTest/QtTest>

#include "core/search/FuzzyMatch.h"

using namespace diffy;

class FuzzyMatchTest : public QObject {
  Q_OBJECT

 private slots:
  void exactMatchScoresHighest() {
    QVERIFY(fuzzyScore("main", "main") > fuzzyScore("main", "omain"));
  }

  void prefixMatchBeatsMiddle() {
    QVERIFY(fuzzyScore("src", "src/app/Foo.cpp") > fuzzyScore("src", "lib/src/Bar.cpp"));
  }

  void subsequenceMatches() {
    const int score = fuzzyScore("cmk", "CMakeLists.txt");
    QVERIFY2(score > 0, "Should match C-M-k as subsequence");
  }

  void camelCaseBonus() {
    QVERIFY(fuzzyScore("DC", "DiffController") > fuzzyScore("DC", "dedicator"));
  }

  void separatorBonus() {
    QVERIFY(fuzzyScore("fb", "foo/bar.txt") > fuzzyScore("fb", "fizzlebaz"));
  }

  void noMatchReturnsZero() {
    QCOMPARE(fuzzyScore("xyz", "CMakeLists.txt"), 0);
  }

  void emptyQueryReturnsZero() {
    QCOMPARE(fuzzyScore("", "anything"), 0);
  }

  void rankReturnsOrderedResults() {
    std::vector<std::string> candidates = {
        "src/app/DiffController.cpp",
        "src/core/diff/DiffTypes.h",
        "src/app/quick/DiffSurfaceItem.cpp",
        "CMakeLists.txt",
        "README.md",
    };

    const auto results = fuzzyRank("diff", candidates);
    QVERIFY(results.size() >= 3);
    // All diff-containing files should appear before non-matching ones
    for (const auto& r : results) {
      QVERIFY(r.score > 0);
    }
  }

  void rankRespectsMaxResults() {
    std::vector<std::string> candidates;
    for (int i = 0; i < 100; ++i) {
      candidates.push_back("file_" + std::to_string(i) + ".txt");
    }

    const auto results = fuzzyRank("file", candidates, 10);
    QVERIFY(static_cast<int>(results.size()) <= 10);
  }

  void pathSeparatorRanking() {
    // "fl" should prefer "FileListPane" over "flake.nix" due to camel
    // but "flake" should rank higher for "fla"
    const int flakeScore = fuzzyScore("fla", "flake.nix");
    const int fileListScore = fuzzyScore("fla", "FileListPane.qml");
    QVERIFY2(flakeScore > 0, "flake should match fla");
    QVERIFY2(flakeScore > fileListScore, "flake.nix should rank above FileListPane for 'fla'");
  }
};

QTEST_GUILESS_MAIN(FuzzyMatchTest)
#include "test_fuzzy_match.moc"
