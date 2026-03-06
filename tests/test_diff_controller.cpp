#include <QFile>
#include <QProcess>
#include <QStandardPaths>
#include <QTemporaryDir>
#include <QDir>
#include <QtTest/QtTest>

#include "app/DiffController.h"
#include "model/DiffRowListModel.h"

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

struct GithubPullRequestFixture {
  QString repoPath;
  QString homePath;
  QString pullRequestUrl;
};

GithubPullRequestFixture initRepositoryWithGithubPullRequest() {
  auto* rootDir = new QTemporaryDir;
  if (!rootDir->isValid()) {
    delete rootDir;
    return {};
  }

  const QString remotePath = rootDir->filePath("origin.git");
  const QString seedPath = rootDir->filePath("seed");
  const QString workPath = rootDir->filePath("work");
  const QString homePath = rootDir->filePath("home");
  QDir().mkpath(seedPath);
  QDir().mkpath(homePath);

  runGit(rootDir->path(), {"init", "--bare", remotePath});
  runGit(rootDir->path(), {"init", seedPath});

  writeFile(QDir(seedPath).filePath("feature.txt"), "base\n");
  runGit(seedPath, {"add", "feature.txt"});
  runGit(seedPath,
         {"-c", "user.name=diffy", "-c", "user.email=diffy@example.com", "commit", "-m", "base"});
  runGit(seedPath, {"branch", "-M", "main"});
  runGit(seedPath, {"remote", "add", "origin", remotePath});
  runGit(seedPath, {"push", "-u", "origin", "main"});

  runGit(seedPath, {"checkout", "-b", "feature/pr-1"});
  writeFile(QDir(seedPath).filePath("feature.txt"), "base\nfeature line\n");
  runGit(seedPath, {"add", "feature.txt"});
  runGit(seedPath,
         {"-c", "user.name=diffy", "-c", "user.email=diffy@example.com", "commit", "-m", "feature"});
  runGit(seedPath, {"push", "origin", "HEAD:refs/pull/1/head"});

  runGit(seedPath, {"checkout", "main"});
  runGit(seedPath,
         {"-c", "user.name=diffy", "-c", "user.email=diffy@example.com", "merge", "--no-ff", "feature/pr-1", "-m",
          "Merge PR 1"});
  runGit(seedPath, {"push", "origin", "HEAD:refs/pull/1/merge"});

  runGit(rootDir->path(), {"clone", remotePath, workPath});
  runGit(workPath, {"remote", "set-url", "origin", "https://github.com/example/diffy.git"});

  writeFile(QDir(homePath).filePath(".gitconfig"),
            QString("[url \"file://%1\"]\n\tinsteadOf = https://github.com/example/diffy.git\n")
                .arg(remotePath));

  return {.repoPath = workPath,
          .homePath = homePath,
          .pullRequestUrl = "https://github.com/example/diffy/pull/1"};
}

bool rowKindsContain(const DiffRowListModel* rows, const QString& kind) {
  if (rows == nullptr) {
    return false;
  }

  for (int rowIndex = 0; rowIndex < rows->rowCount(); ++rowIndex) {
    if (rows->index(rowIndex, 0).data(DiffRowListModel::KindRole).toString() == kind) {
      return true;
    }
  }
  return false;
}

bool rowsContainType(const DiffRowListModel* rows, const QString& rowType) {
  if (rows == nullptr) {
    return false;
  }

  for (int rowIndex = 0; rowIndex < rows->rowCount(); ++rowIndex) {
    if (rows->index(rowIndex, 0).data(DiffRowListModel::RowTypeRole).toString() == rowType) {
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

    const auto* rows = qobject_cast<DiffRowListModel*>(controller.selectedFileRowsModel());
    QVERIFY(rows != nullptr);
    QVERIFY(rows->rowCount() > 0);
    QCOMPARE(controller.selectedFileRowCount(), rows->rowCount());
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
    QCOMPARE(controller.selectedFileRowCount(), 0);
  }

  void compareSupportsGithubPullRequestUrl() {
    const GithubPullRequestFixture fixture = initRepositoryWithGithubPullRequest();
    QVERIFY(!fixture.repoPath.isEmpty());
    QVERIFY(!fixture.homePath.isEmpty());

    const QByteArray previousHome = qgetenv("HOME");
    QVERIFY(qputenv("HOME", fixture.homePath.toUtf8()));

    DiffController controller;
    QVERIFY(controller.openRepository(fixture.repoPath));
    controller.setLeftRef(fixture.pullRequestUrl);
    controller.setRightRef(QString());
    controller.compare();

    if (previousHome.isEmpty()) {
      qunsetenv("HOME");
    } else {
      QVERIFY(qputenv("HOME", previousHome));
    }

    QVERIFY2(controller.errorMessage().isEmpty(), qPrintable(controller.errorMessage()));
    QCOMPARE(controller.compareMode(), QString("three-dot"));
    QCOMPARE(controller.files().size(), 1);
    QCOMPARE(controller.selectedFile().value("path").toString(), QString("feature.txt"));
    QVERIFY(controller.selectedFileRowCount() > 0);
  }
};

QTEST_GUILESS_MAIN(DiffControllerTest)
#include "test_diff_controller.moc"
