#include <QtTest/QtTest>

#include "core/UnifiedDiffParser.h"

using namespace diffy;

class UnifiedDiffParserTest : public QObject {
  Q_OBJECT

 private slots:
  void parsesSingleFilePatch() {
    const std::string patch =
        "diff --git a/src/a.cpp b/src/a.cpp\n"
        "index 111..222 100644\n"
        "--- a/src/a.cpp\n"
        "+++ b/src/a.cpp\n"
        "@@ -1,3 +1,4 @@\n"
        " int a = 1;\n"
        "-int b = 2;\n"
        "+int b = 3;\n"
        "+int c = 4;\n"
        " return a + b;\n";

    UnifiedDiffParser parser;
    DiffDocument doc = parser.parse("left", "right", patch);

    QCOMPARE(doc.files.size(), 1);
    const FileDiff file = doc.files.front();
    QCOMPARE(file.path, std::string("src/a.cpp"));
    QCOMPARE(file.hunks.size(), 1);
    QCOMPARE(file.additions, 2);
    QCOMPARE(file.deletions, 1);

    const Hunk hunk = file.hunks.front();
    QCOMPARE(hunk.lines.size(), 5);
    QCOMPARE(hunk.lines.at(1).kind, LineKind::Deletion);
    QCOMPARE(hunk.lines.at(2).kind, LineKind::Addition);
  }
};

QTEST_MAIN(UnifiedDiffParserTest)
#include "test_unified_diff_parser.moc"
