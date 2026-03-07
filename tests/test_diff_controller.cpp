#include <memory>

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

  std::unique_ptr<QTemporaryDir> configDir_;

 private slots:
  void initTestCase() {
    QVERIFY2(!QStandardPaths::findExecutable("git").isEmpty(), "git is required for diff controller tests");
  }

  void init() {
    configDir_.reset(new QTemporaryDir);
    QVERIFY(configDir_->isValid());
    QVERIFY(qputenv("XDG_CONFIG_HOME", configDir_->path().toUtf8()));
    QSettings::setPath(QSettings::NativeFormat, QSettings::UserScope, configDir_->path());
    QSettings::setPath(QSettings::IniFormat, QSettings::UserScope, configDir_->path());
  }

  void cleanup() {
    configDir_.reset();
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

  void startupViewIsWelcomeWithoutRepo() {
    DiffController controller;
    QCOMPARE(controller.currentView(), QString("welcome"));
  }

  void startupViewIsCompareWithRestoredRepo() {
    const QString repoPath = initRepositoryWithDiff();
    QVERIFY(!repoPath.isEmpty());

    {
      DiffController controller;
      QVERIFY(controller.openRepository(repoPath));
      QCOMPARE(controller.currentView(), QString("compare"));
    }

    DiffController controller2;
    QCOMPARE(controller2.currentView(), QString("compare"));
    QCOMPARE(controller2.repoPath(), repoPath);
  }

  void openRepositoryTransitionsToCompare() {
    const QString repoPath = initRepositoryWithDiff();
    QVERIFY(!repoPath.isEmpty());

    DiffController controller;
    QCOMPARE(controller.currentView(), QString("welcome"));
    QVERIFY(controller.openRepository(repoPath));
    QCOMPARE(controller.currentView(), QString("compare"));
  }

  void compareTransitionsToDiff() {
    const QString repoPath = initRepositoryWithDiff();
    QVERIFY(!repoPath.isEmpty());

    DiffController controller;
    QVERIFY(controller.openRepository(repoPath));
    QCOMPARE(controller.currentView(), QString("compare"));

    controller.setLeftRef("HEAD~1");
    controller.setRightRef("HEAD");
    controller.compare();
    QCOMPARE(controller.currentView(), QString("diff"));
  }

  void goBackTransitions() {
    const QString repoPath = initRepositoryWithDiff();
    QVERIFY(!repoPath.isEmpty());

    DiffController controller;
    QVERIFY(controller.openRepository(repoPath));
    controller.setLeftRef("HEAD~1");
    controller.setRightRef("HEAD");
    controller.compare();
    QCOMPARE(controller.currentView(), QString("diff"));

    controller.goBack();
    QCOMPARE(controller.currentView(), QString("compare"));

    controller.goBack();
    QCOMPARE(controller.currentView(), QString("welcome"));

    controller.goBack();
    QCOMPARE(controller.currentView(), QString("welcome"));
  }

  void recentRepositoriesPersistAndDedup() {
    const QString repoA = initRepositoryWithDiff();
    const QString repoB = initRepositoryWithoutDiff();
    QVERIFY(!repoA.isEmpty());
    QVERIFY(!repoB.isEmpty());

    {
      DiffController controller;
      QVERIFY(controller.recentRepositories().isEmpty());

      QVERIFY(controller.openRepository(repoA));
      QCOMPARE(controller.recentRepositories().size(), 1);
      QCOMPARE(controller.recentRepositories().first(), repoA);

      QVERIFY(controller.openRepository(repoB));
      QCOMPARE(controller.recentRepositories().size(), 2);
      QCOMPARE(controller.recentRepositories().first(), repoB);

      QVERIFY(controller.openRepository(repoA));
      QCOMPARE(controller.recentRepositories().size(), 2);
      QCOMPARE(controller.recentRepositories().first(), repoA);
    }

    DiffController controller2;
    QCOMPARE(controller2.recentRepositories().size(), 2);
    QCOMPARE(controller2.recentRepositories().first(), repoA);
  }

  void displayRefsAbbreviateShas() {
    const QString repoPath = initRepositoryWithDiff();
    QVERIFY(!repoPath.isEmpty());

    QProcess revParse;
    revParse.setProgram("git");
    revParse.setWorkingDirectory(repoPath);
    revParse.setArguments({"rev-parse", "HEAD"});
    revParse.start();
    QVERIFY(revParse.waitForFinished(30000));
    QCOMPARE(revParse.exitCode(), 0);
    const QString headSha = QString::fromUtf8(revParse.readAllStandardOutput()).trimmed();
    QCOMPARE(headSha.size(), 40);

    DiffController controller;
    QVERIFY(controller.openRepository(repoPath));
    controller.setLeftRef(headSha);

    const QString display = controller.leftRefDisplay();
    QVERIFY2(display != headSha, qPrintable("Expected abbreviated ref, got raw SHA"));
    QVERIFY2(display.size() < 40, qPrintable("Expected short display, got: " + display));
  }

  void displayRefsPassthroughBranchNames() {
    const QString repoPath = initRepositoryWithDiff();
    QVERIFY(!repoPath.isEmpty());

    DiffController controller;
    QVERIFY(controller.openRepository(repoPath));
    controller.setLeftRef("HEAD~1");

    QCOMPARE(controller.leftRefDisplay(), QString("HEAD~1"));
  }

  void compareWithThreeDotMode() {
    const QString repoPath = initRepositoryWithDiff();
    QVERIFY(!repoPath.isEmpty());

    DiffController controller;
    QVERIFY(controller.openRepository(repoPath));
    controller.setLeftRef("HEAD~1");
    controller.setRightRef("HEAD");
    controller.setCompareMode("three-dot");
    controller.compare();

    QVERIFY2(controller.errorMessage().isEmpty(), qPrintable(controller.errorMessage()));
    QCOMPARE(controller.compareMode(), QString("three-dot"));
    QVERIFY(controller.files().size() >= 1);
    QVERIFY(controller.selectedFileRowCount() > 0);
  }
};

QTEST_GUILESS_MAIN(DiffControllerTest)
#include "test_diff_controller.moc"
