#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QImage>
#include <QProcess>
#include <QRegularExpression>
#include <QSet>
#include <QTemporaryDir>
#include <QUuid>
#include <QtTest/QtTest>

#include <algorithm>

#include "test_app_smoke.h"

namespace {

void writeFile(const QString& path, const QString& contents) {
  QFile file(path);
  QVERIFY2(file.open(QIODevice::WriteOnly | QIODevice::Truncate | QIODevice::Text),
           qPrintable(file.errorString()));
  file.write(contents.toUtf8());
  file.close();
}

void runGit(const QString& repoPath, const QStringList& arguments) {
  QProcess process;
  process.setProgram("git");
  process.setWorkingDirectory(repoPath);
  process.setArguments(arguments);
  process.start();
  QVERIFY2(process.waitForFinished(30000), "git command timed out");
  const QString stderrText = QString::fromUtf8(process.readAllStandardError()).trimmed();
  QCOMPARE(process.exitCode(), 0);
  QVERIFY2(process.exitStatus() == QProcess::NormalExit, qPrintable(stderrText));
}

QString initRepositoryWithMultipleDiffs() {
  auto* repoDir = new QTemporaryDir;
  if (!repoDir->isValid()) {
    delete repoDir;
    return {};
  }

  runGit(repoDir->path(), {"init"});
  QDir().mkpath(repoDir->filePath("src"));
  QDir().mkpath(repoDir->filePath("include"));

  writeFile(repoDir->filePath("src/example.cpp"),
            "#include <string>\n\nint add(int a, int b) {\n  return a + b;\n}\n");
  writeFile(repoDir->filePath("include/example.h"),
            "#pragma once\n\nint add(int a, int b);\n");
  runGit(repoDir->path(), {"add", "src/example.cpp", "include/example.h"});
  runGit(repoDir->path(),
         {"-c", "user.name=diffy", "-c", "user.email=diffy@example.com", "commit", "-m", "initial"});

  writeFile(repoDir->filePath("src/example.cpp"),
            "#include <string>\n#include <vector>\n\nint add(int a, int b) {\n  const int total = a + b;\n"
            "  return total;\n}\n\nint sub(int a, int b) {\n  return a - b;\n}\n");
  writeFile(repoDir->filePath("include/example.h"),
            "#pragma once\n\nint add(int a, int b);\nint sub(int a, int b);\n");
  runGit(repoDir->path(), {"add", "src/example.cpp", "include/example.h"});
  runGit(repoDir->path(),
         {"-c", "user.name=diffy", "-c", "user.email=diffy@example.com", "commit", "-m", "update"});

  return repoDir->path();
}

QString buildLargeSource(bool updated) {
  QString contents = "int main() {\n";
  for (int i = 1; i <= 220; ++i) {
    if (updated) {
      contents += QString("  int value_%1 = %2;\n").arg(i).arg(i * 2);
    } else {
      contents += QString("  int value_%1 = %1;\n").arg(i);
    }
  }
  contents += "  return 0;\n}\n";
  return contents;
}

QString buildLongLineSource(bool updated) {
  const QChar fill = updated ? QChar('b') : QChar('a');
  QString contents = "#include <string>\n\n";
  contents += QString("const char* payload = \"%1\";\n").arg(QString(fill).repeated(4096));
  contents += "int width() {\n";
  contents += updated ? "  return 2;\n" : "  return 1;\n";
  contents += "}\n";
  return contents;
}

QString initRepositoryWithTallDiff() {
  auto* repoDir = new QTemporaryDir;
  if (!repoDir->isValid()) {
    delete repoDir;
    return {};
  }

  runGit(repoDir->path(), {"init"});
  QDir().mkpath(repoDir->filePath("src"));

  writeFile(repoDir->filePath("src/huge.cpp"), buildLargeSource(false));
  runGit(repoDir->path(), {"add", "src/huge.cpp"});
  runGit(repoDir->path(),
         {"-c", "user.name=diffy", "-c", "user.email=diffy@example.com", "commit", "-m", "initial"});

  writeFile(repoDir->filePath("src/huge.cpp"), buildLargeSource(true));
  runGit(repoDir->path(), {"add", "src/huge.cpp"});
  runGit(repoDir->path(),
         {"-c", "user.name=diffy", "-c", "user.email=diffy@example.com", "commit", "-m", "update"});

  return repoDir->path();
}

QString initRepositoryWithLongChangedLine() {
  auto* repoDir = new QTemporaryDir;
  if (!repoDir->isValid()) {
    delete repoDir;
    return {};
  }

  runGit(repoDir->path(), {"init"});
  QDir().mkpath(repoDir->filePath("src"));

  writeFile(repoDir->filePath("src/long.cpp"), buildLongLineSource(false));
  runGit(repoDir->path(), {"add", "src/long.cpp"});
  runGit(repoDir->path(),
         {"-c", "user.name=diffy", "-c", "user.email=diffy@example.com", "commit", "-m", "initial"});

  writeFile(repoDir->filePath("src/long.cpp"), buildLongLineSource(true));
  runGit(repoDir->path(), {"add", "src/long.cpp"});
  runGit(repoDir->path(),
         {"-c", "user.name=diffy", "-c", "user.email=diffy@example.com", "commit", "-m", "update"});

  return repoDir->path();
}

QString initRepositoryWithDeletedFileDiff() {
  auto* repoDir = new QTemporaryDir;
  if (!repoDir->isValid()) {
    delete repoDir;
    return {};
  }

  runGit(repoDir->path(), {"init"});
  QDir().mkpath(repoDir->filePath("src"));

  writeFile(repoDir->filePath("src/deleted.txt"), "line1\nline2\nline3\nline4\nline5\n");
  runGit(repoDir->path(), {"add", "src/deleted.txt"});
  runGit(repoDir->path(),
         {"-c", "user.name=diffy", "-c", "user.email=diffy@example.com", "commit", "-m", "initial"});

  runGit(repoDir->path(), {"rm", "src/deleted.txt"});
  runGit(repoDir->path(),
         {"-c", "user.name=diffy", "-c", "user.email=diffy@example.com", "commit", "-m", "delete"});

  return repoDir->path();
}

QString diffyBinaryPath() {
  const QDir testsDir(QCoreApplication::applicationDirPath());
  const QString candidate = QFileInfo(testsDir.filePath("../diffy")).absoluteFilePath();
  return QFileInfo(candidate).canonicalFilePath();
}

struct SmokeResult {
  int exitCode = -1;
  QString stdoutText;
  QString stderrText;
  QString capturePath;
};

SmokeResult runDiffySmoke(const QString& repoPath, const QStringList& extraEnv) {
  SmokeResult result;
  QProcess process;
  QTemporaryDir configDir;
  if (!configDir.isValid()) {
    result.stderrText = "failed to create smoke config dir";
    return result;
  }
  QProcessEnvironment env = QProcessEnvironment::systemEnvironment();
  result.capturePath = QDir::temp().filePath(QString("diffy-smoke-%1.png").arg(QUuid::createUuid().toString(QUuid::WithoutBraces)));
  env.insert("QT_QPA_PLATFORM", "offscreen");
  env.insert("QT_QUICK_BACKEND", "software");
  env.insert("XDG_CONFIG_HOME", configDir.path());
  env.insert("XDG_DATA_HOME", configDir.filePath("data"));
  env.insert("XDG_CACHE_HOME", configDir.filePath("cache"));
  env.insert("DIFFY_START_REPO", repoPath);
  env.insert("DIFFY_START_LEFT", "HEAD~1");
  env.insert("DIFFY_START_RIGHT", "HEAD");
  env.insert("DIFFY_START_COMPARE", "1");
  env.insert("DIFFY_REQUIRE_RESULTS", "1");
  env.insert("DIFFY_PRINT_STATE", "1");
  env.insert("DIFFY_FATAL_RUNTIME_WARNINGS", "1");
  env.insert("DIFFY_CAPTURE_PATH", result.capturePath);
  env.insert("DIFFY_EXIT_AFTER_MS", "1800");
  for (const QString& entry : extraEnv) {
    const int separator = entry.indexOf('=');
    env.insert(entry.left(separator), entry.mid(separator + 1));
  }
  process.setProcessEnvironment(env);
  process.setProgram(diffyBinaryPath());
  process.start();

  if (!process.waitForFinished(30000)) {
    process.kill();
    process.waitForFinished();
    result.stderrText = "diffy smoke test timed out";
    return result;
  }
  result.exitCode = process.exitCode();
  result.stdoutText = QString::fromUtf8(process.readAllStandardOutput());
  result.stderrText = QString::fromUtf8(process.readAllStandardError());
  return result;
}

QVariantMap parseStateMatch(const QRegularExpressionMatch& match) {
  return QVariantMap{{"currentView", match.captured(1)},
                     {"files", match.captured(2).toInt()},
                     {"rows", match.captured(3).toInt()},
                     {"selected", match.captured(4).toInt()},
                     {"layout", match.captured(5)},
                     {"surfaceHeight", match.captured(6).toDouble()},
                     {"surfaceWidth", match.captured(7).toDouble()},
                     {"itemWidth", match.captured(8).toDouble()},
                     {"itemHeight", match.captured(9).toDouble()},
                     {"viewportY", match.captured(10).toDouble()},
                     {"displayRows", match.captured(11).toInt()},
                     {"paintCount", match.captured(12).toInt()},
                     {"tileCacheHits", match.captured(13).toInt()},
                     {"tileCacheMisses", match.captured(14).toInt()},
                     {"textureUploads", match.captured(15).toInt()},
                     {"residentTiles", match.captured(16).toInt()},
                     {"pendingTileJobs", match.captured(17).toInt()},
                     {"lastPaintMs", match.captured(18).toDouble()},
                     {"lastRasterMs", match.captured(19).toDouble()},
                     {"lastUploadMs", match.captured(20).toDouble()},
                     {"lastRowsRebuildMs", match.captured(21).toDouble()},
                     {"lastDisplayRowsRebuildMs", match.captured(22).toDouble()},
                     {"lastMetricsMs", match.captured(23).toDouble()},
                     {"pickerVisible", match.captured(24).toInt()},
                     {"error", match.captured(25).trimmed()}};
}

QList<QVariantMap> parseStateLines(const QString& stdoutText) {
  const QRegularExpression linePattern(
      R"(DIFFY_STATE current_view=([^\s]+) files=(\d+) rows=(\d+) selected=(-?\d+) layout=([^\s]+) surface_height=([0-9.-]+) surface_width=([0-9.-]+) item_width=([0-9.-]+) item_height=([0-9.-]+) viewport_y=([0-9.-]+) display_rows=(-?\d+) paint_count=(-?\d+) tile_cache_hits=(-?\d+) tile_cache_misses=(-?\d+) texture_uploads=(-?\d+) resident_tiles=(-?\d+) pending_tile_jobs=(-?\d+) last_paint_ms=([0-9.-]+) last_raster_ms=([0-9.-]+) last_upload_ms=([0-9.-]+) last_rows_rebuild_ms=([0-9.-]+) last_display_rows_rebuild_ms=([0-9.-]+) last_metrics_ms=([0-9.-]+) picker_visible=(\d+) error=(.+))");
  QList<QVariantMap> states;
  auto matchIterator = linePattern.globalMatch(stdoutText);
  while (matchIterator.hasNext()) {
    states.push_back(parseStateMatch(matchIterator.next()));
  }
  return states;
}

QVariantMap parseStateLine(const QString& stdoutText) {
  const QList<QVariantMap> states = parseStateLines(stdoutText);
  return states.isEmpty() ? QVariantMap{} : states.constFirst();
}

void verifyTimingMetrics(const QVariantMap& state) {
  QVERIFY(state.value("lastPaintMs").toDouble() >= 0.0);
  QVERIFY(state.value("lastRasterMs").toDouble() >= 0.0);
  QVERIFY(state.value("lastUploadMs").toDouble() >= 0.0);
  QVERIFY(state.value("lastRowsRebuildMs").toDouble() >= 0.0);
  QVERIFY(state.value("lastDisplayRowsRebuildMs").toDouble() >= 0.0);
  QVERIFY(state.value("lastMetricsMs").toDouble() >= 0.0);
}

int sampleRegionDiversity(const QImage& image, int xStart, int xEnd, int yStart, int yEnd) {
  QSet<QRgb> colors;
  for (int y = yStart; y < yEnd; y += 4) {
    for (int x = xStart; x < xEnd; x += 4) {
      colors.insert(image.pixel(x, y));
    }
  }
  return colors.size();
}

int diffRegionColorDiversity(const QString& imagePath) {
  QImage image(imagePath);
  if (image.isNull()) {
    return 0;
  }

  const int bodyTop = image.height() * 18 / 100;
  const int bodyMid = image.height() * 48 / 100;
  const int bodyBottom = image.height() * 70 / 100;

  const int gutterAndText = sampleRegionDiversity(image, image.width() * 16 / 100, image.width() * 34 / 100,
                                                  bodyTop, bodyMid);
  const int midText = sampleRegionDiversity(image, image.width() * 24 / 100, image.width() * 46 / 100,
                                            bodyMid, bodyBottom);
  const int splitRight = sampleRegionDiversity(image, image.width() * 64 / 100, image.width() * 86 / 100,
                                               bodyTop, bodyMid);
  return std::max({gutterAndText, midText, splitRight});
}

int splitTopHunkDiversity(const QString& imagePath) {
  QImage image(imagePath);
  if (image.isNull()) {
    return 0;
  }

  return sampleRegionDiversity(image, image.width() * 0 / 100, image.width() * 18 / 100,
                               image.height() * 4 / 100, image.height() * 6 / 100);
}

QRgb samplePixelAtPercent(const QString& imagePath, int xPercent, int yPercent) {
  QImage image(imagePath);
  if (image.isNull()) {
    return qRgb(0, 0, 0);
  }

  const int x = std::clamp(image.width() * xPercent / 100, 0, image.width() - 1);
  const int y = std::clamp(image.height() * yPercent / 100, 0, image.height() - 1);
  return image.pixel(x, y);
}

}  // namespace

void AppSmokeTest::initTestCase() {
    QVERIFY2(QFileInfo(diffyBinaryPath()).exists(), "diffy binary must exist");
}

void AppSmokeTest::launchesUnifiedAndPrintsSurfaceState() {
    const QString repoPath = initRepositoryWithMultipleDiffs();
    QVERIFY(!repoPath.isEmpty());

    const SmokeResult result = runDiffySmoke(repoPath, {});
    QVERIFY2(result.stderrText != "diffy smoke test timed out", qPrintable(result.stderrText));
    QCOMPARE(result.exitCode, 0);
    QVERIFY2(result.stderrText.trimmed().isEmpty(), qPrintable(result.stderrText));

    const QVariantMap state = parseStateLine(result.stdoutText);
    QVERIFY2(!state.isEmpty(), qPrintable(result.stdoutText));
    QCOMPARE(state.value("currentView").toString(), QString("diff"));
    QVERIFY(state.value("files").toInt() >= 2);
    QVERIFY(state.value("rows").toInt() > 0);
    QCOMPARE(state.value("layout").toString(), QString("unified"));
    QVERIFY(state.value("surfaceHeight").toDouble() > 0.0);
    QVERIFY(state.value("surfaceWidth").toDouble() > 0.0);
    QVERIFY(state.value("itemWidth").toDouble() > 0.0);
    QVERIFY(state.value("itemHeight").toDouble() > 0.0);
    QVERIFY(state.value("displayRows").toInt() > 0);
    QVERIFY(state.value("paintCount").toInt() > 0);
    QVERIFY(state.value("tileCacheHits").toInt() >= 0);
    QVERIFY(state.value("tileCacheMisses").toInt() >= 0);
    QVERIFY(state.value("textureUploads").toInt() >= 0);
    QVERIFY(state.value("residentTiles").toInt() >= 0);
    verifyTimingMetrics(state);
    QCOMPARE(state.value("pendingTileJobs").toInt(), 0);
    QVERIFY2(QFileInfo::exists(result.capturePath), qPrintable(result.capturePath));
    QVERIFY(state.value("textureUploads").toInt() > 0);
    QVERIFY(state.value("residentTiles").toInt() > 0);
    QCOMPARE(state.value("error").toString(), QString("none"));
}

void AppSmokeTest::launchesSplitSecondFileAndPrintsSurfaceState() {
    const QString repoPath = initRepositoryWithMultipleDiffs();
    QVERIFY(!repoPath.isEmpty());

    const SmokeResult result = runDiffySmoke(repoPath, {"DIFFY_START_LAYOUT=split", "DIFFY_START_FILE_INDEX=1"});
    QVERIFY2(result.stderrText != "diffy smoke test timed out", qPrintable(result.stderrText));
    QCOMPARE(result.exitCode, 0);
    QVERIFY2(result.stderrText.trimmed().isEmpty(), qPrintable(result.stderrText));

    const QVariantMap state = parseStateLine(result.stdoutText);
    QVERIFY2(!state.isEmpty(), qPrintable(result.stdoutText));
    QCOMPARE(state.value("currentView").toString(), QString("diff"));
    QVERIFY(state.value("files").toInt() >= 2);
    QVERIFY(state.value("rows").toInt() > 0);
    QCOMPARE(state.value("selected").toInt(), 1);
    QCOMPARE(state.value("layout").toString(), QString("split"));
    QVERIFY(state.value("surfaceHeight").toDouble() > 0.0);
    QVERIFY(state.value("surfaceWidth").toDouble() > 0.0);
    QVERIFY(state.value("itemWidth").toDouble() > 0.0);
    QVERIFY(state.value("itemHeight").toDouble() > 0.0);
    QVERIFY(state.value("displayRows").toInt() > 0);
    QVERIFY(state.value("paintCount").toInt() > 0);
    QVERIFY(state.value("residentTiles").toInt() > 0);
    QVERIFY(state.value("textureUploads").toInt() > 0);
    verifyTimingMetrics(state);
    QCOMPARE(state.value("pendingTileJobs").toInt(), 0);
    QVERIFY2(QFileInfo::exists(result.capturePath), qPrintable(result.capturePath));
    QVERIFY2(splitTopHunkDiversity(result.capturePath) > 2, qPrintable(result.capturePath));
    QCOMPARE(state.value("error").toString(), QString("none"));
}

void AppSmokeTest::scrollsUnifiedViewportWithoutShrinkingSurface() {
    const QString repoPath = initRepositoryWithMultipleDiffs();
    QVERIFY(!repoPath.isEmpty());

    const SmokeResult result = runDiffySmoke(repoPath, {"DIFFY_START_SCROLL_Y=220"});
    QVERIFY2(result.stderrText != "diffy smoke test timed out", qPrintable(result.stderrText));
    QCOMPARE(result.exitCode, 0);
    QVERIFY2(result.stderrText.trimmed().isEmpty(), qPrintable(result.stderrText));

    const QVariantMap state = parseStateLine(result.stdoutText);
    QVERIFY2(!state.isEmpty(), qPrintable(result.stdoutText));
    QCOMPARE(state.value("currentView").toString(), QString("diff"));
    QCOMPARE(state.value("layout").toString(), QString("unified"));
    QVERIFY(state.value("surfaceHeight").toDouble() > 100.0);
    QVERIFY(state.value("itemHeight").toDouble() > 100.0);
    verifyTimingMetrics(state);
    QVERIFY2(QFileInfo::exists(result.capturePath), qPrintable(result.capturePath));
}

void AppSmokeTest::wheelScrollsSplitViewportDespiteHorizontalTrackpadNoise() {
    const QString repoPath = initRepositoryWithTallDiff();
    QVERIFY(!repoPath.isEmpty());

    const SmokeResult result =
        runDiffySmoke(repoPath, {"DIFFY_START_LAYOUT=split", "DIFFY_START_FILE_INDEX=0",
                                 "DIFFY_START_WHEEL_PIXEL_X=2", "DIFFY_START_WHEEL_PIXEL_Y=-60",
                                 "DIFFY_PRINT_STATE_DELAY_MS=260"});
    QVERIFY2(result.stderrText != "diffy smoke test timed out", qPrintable(result.stderrText));
    QCOMPARE(result.exitCode, 0);
    QVERIFY2(result.stderrText.trimmed().isEmpty(), qPrintable(result.stderrText));

    const QVariantMap state = parseStateLine(result.stdoutText);
    QVERIFY2(!state.isEmpty(), qPrintable(result.stdoutText));
    QCOMPARE(state.value("layout").toString(), QString("split"));
    QVERIFY(state.value("surfaceHeight").toDouble() > state.value("itemHeight").toDouble());
    QVERIFY(state.value("viewportY").toDouble() >= 40.0);
    QVERIFY(state.value("residentTiles").toInt() > 0);
    QVERIFY(state.value("textureUploads").toInt() > 0);
    verifyTimingMetrics(state);
    QCOMPARE(state.value("pendingTileJobs").toInt(), 0);
}

void AppSmokeTest::switchesFromSplitToUnifiedWhileScrolled() {
    const QString repoPath = initRepositoryWithTallDiff();
    QVERIFY(!repoPath.isEmpty());

    const SmokeResult result =
        runDiffySmoke(repoPath, {"DIFFY_START_LAYOUT=split",
                                 "DIFFY_START_FILE_INDEX=0",
                                 "DIFFY_START_SCROLL_Y=1200",
                                 "DIFFY_SWITCH_LAYOUT_TO=unified",
                                 "DIFFY_SWITCH_LAYOUT_AFTER_MS=260",
                                 "DIFFY_PRINT_STATE_DELAY_MS=900",
                                 "DIFFY_CAPTURE_DELAY_MS=980",
                                 "DIFFY_EXIT_AFTER_MS=1250"});
    QVERIFY2(result.stderrText != "diffy smoke test timed out", qPrintable(result.stderrText));
    QCOMPARE(result.exitCode, 0);
    QVERIFY2(result.stderrText.trimmed().isEmpty(), qPrintable(result.stderrText));

    const QVariantMap state = parseStateLine(result.stdoutText);
    QVERIFY2(!state.isEmpty(), qPrintable(result.stdoutText));
    QCOMPARE(state.value("layout").toString(), QString("unified"));
    QVERIFY(state.value("viewportY").toDouble() >= 1000.0);
    QVERIFY(state.value("paintCount").toInt() > 0);
    QVERIFY(state.value("residentTiles").toInt() > 0);
    QVERIFY(state.value("textureUploads").toInt() > 0);
    verifyTimingMetrics(state);
    QCOMPARE(state.value("pendingTileJobs").toInt(), 0);
    QVERIFY2(QFileInfo::exists(result.capturePath), qPrintable(result.capturePath));
}

void AppSmokeTest::warmSplitReverseWheelDoesNotUploadMoreTextures() {
    const QString repoPath = initRepositoryWithTallDiff();
    QVERIFY(!repoPath.isEmpty());

    const SmokeResult result =
        runDiffySmoke(repoPath, {"DIFFY_START_LAYOUT=split",
                                 "DIFFY_START_FILE_INDEX=0",
                                 "DIFFY_START_WHEEL_PIXEL_Y=-60",
                                 "DIFFY_START_SECOND_WHEEL_PIXEL_Y=60",
                                 "DIFFY_START_SECOND_WHEEL_AFTER_MS=240",
                                 "DIFFY_PRINT_STATE_DELAY_MS=180",
                                 "DIFFY_PRINT_STATE_REPEAT_MS=180",
                                 "DIFFY_PRINT_STATE_COUNT=2"});
    QVERIFY2(result.stderrText != "diffy smoke test timed out", qPrintable(result.stderrText));
    QCOMPARE(result.exitCode, 0);
    QVERIFY2(result.stderrText.trimmed().isEmpty(), qPrintable(result.stderrText));

    const QList<QVariantMap> states = parseStateLines(result.stdoutText);
    QCOMPARE(states.size(), 2);
    QCOMPARE(states.at(0).value("layout").toString(), QString("split"));
    QCOMPARE(states.at(1).value("layout").toString(), QString("split"));
    QVERIFY(states.at(0).value("residentTiles").toInt() > 0);
    verifyTimingMetrics(states.at(0));
    verifyTimingMetrics(states.at(1));
    QCOMPARE(states.at(1).value("textureUploads").toInt(), states.at(0).value("textureUploads").toInt());
    QCOMPARE(states.at(1).value("pendingTileJobs").toInt(), 0);
}

void AppSmokeTest::switchesFilesAndKeepsTimingMetricsAvailable() {
    const QString repoPath = initRepositoryWithMultipleDiffs();
    QVERIFY(!repoPath.isEmpty());

    const SmokeResult result =
        runDiffySmoke(repoPath, {"DIFFY_START_FILE_PATH=include/example.h",
                                 "DIFFY_SWITCH_FILE_TO_PATH=src/example.cpp",
                                 "DIFFY_SWITCH_FILE_AFTER_MS=220",
                                 "DIFFY_PRINT_STATE_DELAY_MS=900",
                                 "DIFFY_CAPTURE_DELAY_MS=980",
                                 "DIFFY_EXIT_AFTER_MS=1250"});
    QVERIFY2(result.stderrText != "diffy smoke test timed out", qPrintable(result.stderrText));
    QCOMPARE(result.exitCode, 0);
    QVERIFY2(result.stderrText.trimmed().isEmpty(), qPrintable(result.stderrText));

    const QVariantMap state = parseStateLine(result.stdoutText);
    QVERIFY2(!state.isEmpty(), qPrintable(result.stdoutText));
    QCOMPARE(state.value("currentView").toString(), QString("diff"));
    QVERIFY(state.value("rows").toInt() >= 8);
    QVERIFY(state.value("paintCount").toInt() > 0);
    verifyTimingMetrics(state);
    QVERIFY2(QFileInfo::exists(result.capturePath), qPrintable(result.capturePath));
}

void AppSmokeTest::longChangedLineSupportsSplitHorizontalScrollAndTimingMetrics() {
    const QString repoPath = initRepositoryWithLongChangedLine();
    QVERIFY(!repoPath.isEmpty());

    const SmokeResult result =
        runDiffySmoke(repoPath, {"DIFFY_START_LAYOUT=split",
                                 "DIFFY_START_FILE_INDEX=0",
                                 "DIFFY_START_WHEEL_PIXEL_X=-160",
                                 "DIFFY_PRINT_STATE_DELAY_MS=260"});
    QVERIFY2(result.stderrText != "diffy smoke test timed out", qPrintable(result.stderrText));
    QCOMPARE(result.exitCode, 0);
    QVERIFY2(result.stderrText.trimmed().isEmpty(), qPrintable(result.stderrText));

    const QVariantMap state = parseStateLine(result.stdoutText);
    QVERIFY2(!state.isEmpty(), qPrintable(result.stdoutText));
    QCOMPARE(state.value("layout").toString(), QString("split"));
    QVERIFY(state.value("rows").toInt() > 0);
    QVERIFY(state.value("paintCount").toInt() > 0);
    QVERIFY(state.value("textureUploads").toInt() > 0);
    verifyTimingMetrics(state);
    QVERIFY2(QFileInfo::exists(result.capturePath), qPrintable(result.capturePath));
}

void AppSmokeTest::splitDeletedFileKeepsSpacerBackgroundInBlankPane() {
    const QString repoPath = initRepositoryWithDeletedFileDiff();
    QVERIFY(!repoPath.isEmpty());

    const SmokeResult result = runDiffySmoke(repoPath, {"DIFFY_START_LAYOUT=split", "DIFFY_START_FILE_INDEX=0"});
    QVERIFY2(result.stderrText != "diffy smoke test timed out", qPrintable(result.stderrText));
    QCOMPARE(result.exitCode, 0);
    QVERIFY2(result.stderrText.trimmed().isEmpty(), qPrintable(result.stderrText));

    const QVariantMap state = parseStateLine(result.stdoutText);
    QVERIFY2(!state.isEmpty(), qPrintable(result.stdoutText));
    QCOMPARE(state.value("layout").toString(), QString("split"));
    verifyTimingMetrics(state);
    QVERIFY2(QFileInfo::exists(result.capturePath), qPrintable(result.capturePath));

    const QRgb rightBlankRow = samplePixelAtPercent(result.capturePath, 72, 11);
    const QRgb leftDeletedRow = samplePixelAtPercent(result.capturePath, 24, 11);
    const QRgb canvasBelow = samplePixelAtPercent(result.capturePath, 72, 35);
    QVERIFY2(rightBlankRow != canvasBelow, qPrintable(result.capturePath));
    QVERIFY2(leftDeletedRow != canvasBelow, qPrintable(result.capturePath));
}

void AppSmokeTest::opensInAppRepositoryPickerWithoutWarnings() {
    const QString repoPath = initRepositoryWithMultipleDiffs();
    QVERIFY(!repoPath.isEmpty());

    const SmokeResult result = runDiffySmoke(repoPath, {"DIFFY_OPEN_REPO_PICKER=1"});
    QVERIFY2(result.stderrText != "diffy smoke test timed out", qPrintable(result.stderrText));
    QCOMPARE(result.exitCode, 0);
    QVERIFY2(result.stderrText.trimmed().isEmpty(), qPrintable(result.stderrText));

    const QVariantMap state = parseStateLine(result.stdoutText);
    QVERIFY2(!state.isEmpty(), qPrintable(result.stdoutText));
    QCOMPARE(state.value("currentView").toString(), QString("diff"));
    QCOMPARE(state.value("pickerVisible").toInt(), 1);
    verifyTimingMetrics(state);
    QVERIFY2(QFileInfo::exists(result.capturePath), qPrintable(result.capturePath));
}

QTEST_GUILESS_MAIN(AppSmokeTest)
