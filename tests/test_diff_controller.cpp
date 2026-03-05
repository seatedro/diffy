#include <QFile>
#include <QProcess>
#include <QStandardPaths>
#include <QTemporaryDir>
#include <QtTest/QtTest>

#include "core/DiffController.h"

using namespace diffy;

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

QString initRepositoryWithDiff() {
  auto* repoDir = new QTemporaryDir;
  if (!repoDir->isValid()) {
    delete repoDir;
    return {};
  }

  runGit(repoDir->path(), {"init"});
  writeFile(repoDir->filePath("notes.txt"), "alpha\nbeta\n");
  runGit(repoDir->path(), {"add", "notes.txt"});
  runGit(repoDir->path(),
         {"-c", "user.name=diffy", "-c", "user.email=diffy@example.com", "commit", "-m", "initial"});

  writeFile(repoDir->filePath("notes.txt"), "alpha\nbeta revised\ncharlie\n");
  runGit(repoDir->path(), {"add", "notes.txt"});
  runGit(repoDir->path(),
         {"-c", "user.name=diffy", "-c", "user.email=diffy@example.com", "commit", "-m", "update"});

  return repoDir->path();
}

QString initRepositoryWithoutDiff() {
  auto* repoDir = new QTemporaryDir;
  if (!repoDir->isValid()) {
    delete repoDir;
    return {};
  }

  runGit(repoDir->path(), {"init"});
  writeFile(repoDir->filePath("readme.txt"), "hello\n");
  runGit(repoDir->path(), {"add", "readme.txt"});
  runGit(repoDir->path(),
         {"-c", "user.name=diffy", "-c", "user.email=diffy@example.com", "commit", "-m", "initial"});

  return repoDir->path();
}

bool rowKindsContain(const QVariantList& rows, const QString& kind) {
  for (const QVariant& rowValue : rows) {
    const QVariantMap row = rowValue.toMap();
    if (row.value("kind").toString() == kind) {
      return true;
    }
  }
  return false;
}

bool rowsContainType(const QVariantList& rows, const QString& rowType) {
  for (const QVariant& rowValue : rows) {
    const QVariantMap row = rowValue.toMap();
    if (row.value("rowType").toString() == rowType) {
      return true;
    }
  }
  return false;
}

}  // namespace

class DiffControllerTest : public QObject {
  Q_OBJECT

 private slots:
  void initTestCase() {
    QVERIFY2(!QStandardPaths::findExecutable("git").isEmpty(), "git is required for diff controller tests");
    QVERIFY(qputenv("XDG_CONFIG_HOME", QByteArray("/tmp/diffy-test-config")));
  }

  void compareProducesVisibleRows() {
    const QString repoPath = initRepositoryWithDiff();
    QVERIFY(!repoPath.isEmpty());

    DiffController controller;
    QVERIFY(controller.openRepository(repoPath));
    controller.setLeftRef("HEAD~1");
    controller.setRightRef("HEAD");
    controller.compare();

    QVERIFY2(controller.errorMessage().isEmpty(), qPrintable(controller.errorMessage()));
    QCOMPARE(controller.files().size(), 1);
    QCOMPARE(controller.selectedFile().value("path").toString(), QString("notes.txt"));

    const QVariantList rows = controller.selectedFileRows();
    QVERIFY(!rows.isEmpty());
    QVERIFY(rowsContainType(rows, "hunk"));
    QVERIFY(rowKindsContain(rows, "add"));
    QVERIFY(rowKindsContain(rows, "del"));
  }

  void openingDifferentRepositoryClearsPreviousComparison() {
    const QString repoWithDiff = initRepositoryWithDiff();
    const QString repoWithoutDiff = initRepositoryWithoutDiff();
    QVERIFY(!repoWithDiff.isEmpty());
    QVERIFY(!repoWithoutDiff.isEmpty());

    DiffController controller;
    QVERIFY(controller.openRepository(repoWithDiff));
    controller.setLeftRef("HEAD~1");
    controller.setRightRef("HEAD");
    controller.compare();
    QVERIFY(!controller.files().isEmpty());

    QVERIFY(controller.openRepository(repoWithoutDiff));
    QCOMPARE(controller.files().size(), 0);
    QCOMPARE(controller.selectedFileIndex(), -1);
    QVERIFY(controller.selectedFile().isEmpty());
    QVERIFY(controller.selectedFileRows().isEmpty());
  }
};

QTEST_GUILESS_MAIN(DiffControllerTest)
#include "test_diff_controller.moc"
