#include <QDir>
#include <QFile>
#include <QProcess>
#include <QTemporaryDir>
#include <QtTest/QtTest>

#include "app/models/RepositoryPickerModel.h"

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

QString createRepoRoot() {
  auto* rootDir = new QTemporaryDir;
  if (!rootDir->isValid()) {
    delete rootDir;
    return {};
  }

  const QString repoPath = rootDir->filePath("repo");
  QDir().mkpath(repoPath);
  runGit(rootDir->path(), {"init", repoPath});
  writeFile(QDir(repoPath).filePath("readme.txt"), "hello\n");
  runGit(repoPath, {"add", "readme.txt"});
  runGit(repoPath,
         {"-c", "user.name=diffy", "-c", "user.email=diffy@example.com", "commit", "-m", "initial"});
  QDir().mkpath(rootDir->filePath("other"));
  return rootDir->path();
}

}  // namespace

class RepositoryPickerModelTest : public QObject {
  Q_OBJECT

 private slots:
  void listsRepositoriesAndNavigates() {
    const QString rootPath = createRepoRoot();
    QVERIFY(!rootPath.isEmpty());

    RepositoryPickerModel model;
    model.setCurrentPath(rootPath);

    QVERIFY(model.rowCount() >= 2);

    int repoIndex = -1;
    int otherIndex = -1;
    for (int row = 0; row < model.rowCount(); ++row) {
      const QModelIndex index = model.index(row, 0);
      const QString name = index.data(RepositoryPickerModel::NameRole).toString();
      if (name == "repo") {
        repoIndex = row;
      } else if (name == "other") {
        otherIndex = row;
      }
    }

    QVERIFY(repoIndex >= 0);
    QVERIFY(otherIndex >= 0);
    QVERIFY(model.entryIsRepository(repoIndex));
    QVERIFY(!model.entryIsRepository(otherIndex));

    QVERIFY(model.navigateToEntry(otherIndex));
    QCOMPARE(model.currentPath(), QDir(rootPath).filePath("other"));
    QVERIFY(!model.currentPathIsRepository());

    QVERIFY(model.goUp());
    QCOMPARE(model.currentPath(), QDir(rootPath).absolutePath());
  }
};

QTEST_GUILESS_MAIN(RepositoryPickerModelTest)
#include "test_repository_picker_model.moc"
