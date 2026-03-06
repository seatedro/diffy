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

QVariantMap parseStateLine(const QString& stdoutText) {
  const QRegularExpression linePattern(
      R"(DIFFY_STATE files=(\d+) rows=(\d+) selected=(-?\d+) layout=([^\s]+) surface_height=([0-9.]+) surface_width=([0-9.]+) item_width=([0-9.]+) item_height=([0-9.]+) display_rows=(\d+) paint_count=(\d+) error=(.+))");
  const QRegularExpressionMatch match = linePattern.match(stdoutText);
  if (!match.hasMatch()) {
    return {};
  }

  return QVariantMap{{"files", match.captured(1).toInt()},
                     {"rows", match.captured(2).toInt()},
                     {"selected", match.captured(3).toInt()},
                     {"layout", match.captured(4)},
                     {"surfaceHeight", match.captured(5).toDouble()},
                     {"surfaceWidth", match.captured(6).toDouble()},
                     {"itemWidth", match.captured(7).toDouble()},
                     {"itemHeight", match.captured(8).toDouble()},
                     {"displayRows", match.captured(9).toInt()},
                     {"paintCount", match.captured(10).toInt()},
                     {"error", match.captured(11).trimmed()}};
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

}  // namespace

class AppSmokeTest : public QObject {
  Q_OBJECT

 private slots:
  void initTestCase() {
    QVERIFY2(QFileInfo(diffyBinaryPath()).exists(), "diffy binary must exist");
  }

  void launchesUnifiedAndPrintsSurfaceState() {
    const QString repoPath = initRepositoryWithMultipleDiffs();
    QVERIFY(!repoPath.isEmpty());

    const SmokeResult result = runDiffySmoke(repoPath, {});
    QVERIFY2(result.stderrText != "diffy smoke test timed out", qPrintable(result.stderrText));
    QCOMPARE(result.exitCode, 0);
    QVERIFY2(result.stderrText.trimmed().isEmpty(), qPrintable(result.stderrText));

    const QVariantMap state = parseStateLine(result.stdoutText);
    QVERIFY2(!state.isEmpty(), qPrintable(result.stdoutText));
    QVERIFY(state.value("files").toInt() >= 2);
    QVERIFY(state.value("rows").toInt() > 0);
    QCOMPARE(state.value("layout").toString(), QString("unified"));
    QVERIFY(state.value("surfaceHeight").toDouble() > 0.0);
    QVERIFY(state.value("surfaceWidth").toDouble() > 0.0);
    QVERIFY(state.value("itemWidth").toDouble() > 0.0);
    QVERIFY(state.value("itemHeight").toDouble() > 0.0);
    QVERIFY(state.value("displayRows").toInt() > 0);
    QVERIFY(state.value("paintCount").toInt() > 0);
    QVERIFY2(QFileInfo::exists(result.capturePath), qPrintable(result.capturePath));
    QVERIFY(diffRegionColorDiversity(result.capturePath) > 3);
    QCOMPARE(state.value("error").toString(), QString("none"));
  }

  void launchesSplitSecondFileAndPrintsSurfaceState() {
    const QString repoPath = initRepositoryWithMultipleDiffs();
    QVERIFY(!repoPath.isEmpty());

    const SmokeResult result = runDiffySmoke(repoPath, {"DIFFY_START_LAYOUT=split", "DIFFY_START_FILE_INDEX=1"});
    QVERIFY2(result.stderrText != "diffy smoke test timed out", qPrintable(result.stderrText));
    QCOMPARE(result.exitCode, 0);
    QVERIFY2(result.stderrText.trimmed().isEmpty(), qPrintable(result.stderrText));

    const QVariantMap state = parseStateLine(result.stdoutText);
    QVERIFY2(!state.isEmpty(), qPrintable(result.stdoutText));
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
    QVERIFY2(QFileInfo::exists(result.capturePath), qPrintable(result.capturePath));
    QVERIFY(diffRegionColorDiversity(result.capturePath) > 3);
    QCOMPARE(state.value("error").toString(), QString("none"));
  }

  void scrollsUnifiedViewportWithoutShrinkingSurface() {
    const QString repoPath = initRepositoryWithMultipleDiffs();
    QVERIFY(!repoPath.isEmpty());

    const SmokeResult result = runDiffySmoke(repoPath, {"DIFFY_START_SCROLL_Y=420"});
    QVERIFY2(result.stderrText != "diffy smoke test timed out", qPrintable(result.stderrText));
    QCOMPARE(result.exitCode, 0);
    QVERIFY2(result.stderrText.trimmed().isEmpty(), qPrintable(result.stderrText));

    const QVariantMap state = parseStateLine(result.stdoutText);
    QVERIFY2(!state.isEmpty(), qPrintable(result.stdoutText));
    QCOMPARE(state.value("layout").toString(), QString("unified"));
    QVERIFY(state.value("surfaceHeight").toDouble() > 100.0);
    QVERIFY(state.value("itemHeight").toDouble() > 100.0);
    QVERIFY2(QFileInfo::exists(result.capturePath), qPrintable(result.capturePath));
    QVERIFY(diffRegionColorDiversity(result.capturePath) > 3);
  }
};

QTEST_GUILESS_MAIN(AppSmokeTest)
#include "test_app_smoke.moc"
