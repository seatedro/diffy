#include <QtTest/QtTest>

#include "core/rendering/DiffLayoutEngine.h"

using namespace diffy;

namespace {

DiffSourceRow makeFileHeader(std::string header = "src/example.cpp") {
  DiffSourceRow row;
  row.rowType = DiffRowType::FileHeader;
  row.header = std::move(header);
  return row;
}

DiffSourceRow makeHunk(std::string header = "@@ -1,2 +1,3 @@") {
  DiffSourceRow row;
  row.rowType = DiffRowType::Hunk;
  row.header = std::move(header);
  return row;
}

DiffSourceRow makeLine(DiffLineKind kind, int oldLine, int newLine, double textWidth) {
  DiffSourceRow row;
  row.rowType = DiffRowType::Line;
  row.kind = kind;
  row.oldLine = oldLine;
  row.newLine = newLine;
  row.textWidth = textWidth;
  return row;
}

}  // namespace

class DiffLayoutEngineTest : public QObject {
  Q_OBJECT

 private slots:
  void unifiedLayoutTracksOffsetsAndWrap() {
    DiffLayoutEngine model;
    model.setSourceRows({
        makeFileHeader(),
        makeHunk(),
        makeLine(DiffLineKind::Context, 98, 98, 20.0),
        makeLine(DiffLineKind::Addition, -1, 1205, 120.0),
    });

    DiffLayoutConfig config;
    config.mode = DiffLayoutMode::Unified;
    config.rowHeight = 10.0;
    config.hunkHeight = 12.0;
    config.fileHeaderHeight = 14.0;

    model.rebuild(config);

    const auto& rows = model.rows();
    QCOMPARE(rows.size(), static_cast<size_t>(4));
    QCOMPARE(model.lineNumberDigits(), 4);
    QCOMPARE(rows.at(0).top, 0.0);
    QCOMPARE(rows.at(0).height, 14.0);
    QCOMPARE(rows.at(1).top, 14.0);
    QCOMPARE(rows.at(1).height, 12.0);
    QCOMPARE(rows.at(2).top, 26.0);
    QCOMPARE(rows.at(2).height, 10.0);
    QCOMPARE(rows.at(3).top, 36.0);
    QCOMPARE(rows.at(3).height, 10.0);
    QCOMPARE(model.contentHeight(), 46.0);
    QCOMPARE(model.rowIndexAtY(0.0), 0);
    QCOMPARE(model.rowIndexAtY(13.9), 0);
    QCOMPARE(model.rowIndexAtY(14.0), 1);
    QCOMPARE(model.rowIndexAtY(45.0), 3);

    config.wrapEnabled = true;
    config.unifiedWrapWidth = 50.0;
    model.rebuild(config);

    const auto& wrappedRows = model.rows();
    QCOMPARE(wrappedRows.at(3).top, 36.0);
    QCOMPARE(wrappedRows.at(3).height, 30.0);
    QCOMPARE(model.contentHeight(), 66.0);
    QCOMPARE(model.rowIndexAtY(60.0), 3);
  }

  void splitLayoutPairsDeleteAddBlocksAndWrapsTallestSide() {
    DiffLayoutEngine model;
    model.setSourceRows({
        makeHunk(),
        makeLine(DiffLineKind::Deletion, 10, -1, 35.0),
        makeLine(DiffLineKind::Deletion, 11, -1, 15.0),
        makeLine(DiffLineKind::Addition, -1, 10, 90.0),
        makeLine(DiffLineKind::Context, 12, 11, 20.0),
    });

    DiffLayoutConfig config;
    config.mode = DiffLayoutMode::Split;
    config.rowHeight = 10.0;
    config.hunkHeight = 12.0;
    config.fileHeaderHeight = 14.0;
    config.wrapEnabled = true;
    config.splitWrapWidth = 40.0;

    model.rebuild(config);

    const auto& rows = model.rows();
    QCOMPARE(rows.size(), static_cast<size_t>(4));

    const DiffDisplayRow& firstChange = rows.at(1);
    QCOMPARE(static_cast<int>(firstChange.leftKind), static_cast<int>(DiffLineKind::Deletion));
    QCOMPARE(static_cast<int>(firstChange.rightKind), static_cast<int>(DiffLineKind::Addition));
    QCOMPARE(firstChange.leftLine, 10);
    QCOMPARE(firstChange.rightLine, 10);
    QCOMPARE(firstChange.height, 30.0);

    const DiffDisplayRow& secondChange = rows.at(2);
    QCOMPARE(static_cast<int>(secondChange.leftKind), static_cast<int>(DiffLineKind::Deletion));
    QCOMPARE(static_cast<int>(secondChange.rightKind), static_cast<int>(DiffLineKind::Spacer));
    QCOMPARE(secondChange.leftLine, 11);
    QCOMPARE(secondChange.rightLine, -1);
    QCOMPARE(secondChange.height, 10.0);

    const DiffDisplayRow& context = rows.at(3);
    QCOMPARE(static_cast<int>(context.leftKind), static_cast<int>(DiffLineKind::Context));
    QCOMPARE(static_cast<int>(context.rightKind), static_cast<int>(DiffLineKind::Context));
    QCOMPARE(context.leftLine, 12);
    QCOMPARE(context.rightLine, 11);
    QCOMPARE(context.height, 10.0);
    QCOMPARE(model.contentHeight(), 62.0);
  }

  void stickyAndHunkNavigationFollowDisplayRows() {
    DiffLayoutEngine model;
    model.setSourceRows({
        makeFileHeader(),
        makeHunk("@@ first @@"),
        makeLine(DiffLineKind::Context, 1, 1, 12.0),
        makeHunk("@@ second @@"),
        makeLine(DiffLineKind::Addition, -1, 2, 18.0),
    });

    DiffLayoutConfig config;
    config.mode = DiffLayoutMode::Unified;
    config.rowHeight = 10.0;
    config.hunkHeight = 12.0;
    config.fileHeaderHeight = 14.0;

    model.rebuild(config);

    QCOMPARE(model.fileHeaderRowIndex(), 0);
    QCOMPARE(model.stickyHunkRowIndexAtY(0.0), -1);
    QCOMPARE(model.stickyHunkRowIndexAtY(14.0), 1);
    QCOMPARE(model.stickyHunkRowIndexAtY(40.0), 3);
    QCOMPARE(model.nextHunkRowIndex(1), 3);
    QCOMPARE(model.nextHunkRowIndex(3), -1);
    QCOMPARE(model.previousHunkRowIndex(4), 3);
    QCOMPARE(model.previousHunkRowIndex(3), 1);
  }

  void prewarmAlternateLayoutDoesNotDisturbActiveLayout() {
    DiffLayoutEngine model;
    model.setSourceRows({
        makeFileHeader(),
        makeHunk(),
        makeLine(DiffLineKind::Deletion, 10, -1, 90.0),
        makeLine(DiffLineKind::Addition, -1, 10, 110.0),
        makeLine(DiffLineKind::Context, 11, 11, 30.0),
    });

    DiffLayoutConfig unified;
    unified.mode = DiffLayoutMode::Unified;
    unified.rowHeight = 10.0;
    unified.hunkHeight = 12.0;
    unified.fileHeaderHeight = 14.0;
    unified.wrapEnabled = true;
    unified.unifiedWrapWidth = 40.0;
    unified.splitWrapWidth = 40.0;

    DiffLayoutConfig split = unified;
    split.mode = DiffLayoutMode::Split;

    model.rebuild(unified);
    const std::vector<DiffDisplayRow> beforeRows = model.rows();
    const double beforeHeight = model.contentHeight();

    model.prewarm(split);

    QCOMPARE(model.rows().size(), beforeRows.size());
    QCOMPARE(model.contentHeight(), beforeHeight);
    QCOMPARE(model.rows().at(0).height, beforeRows.at(0).height);
    QCOMPARE(model.rows().at(1).height, beforeRows.at(1).height);

    model.rebuild(split);
    QCOMPARE(model.rows().size(), static_cast<size_t>(4));
    QCOMPARE(static_cast<int>(model.rows().at(2).leftKind), static_cast<int>(DiffLineKind::Deletion));
    QCOMPARE(static_cast<int>(model.rows().at(2).rightKind), static_cast<int>(DiffLineKind::Addition));
  }
};

QTEST_GUILESS_MAIN(DiffLayoutEngineTest)
#include "test_diff_layout_engine.moc"
