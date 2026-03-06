#include <QBuffer>
#include <QFile>
#include <QProcess>
#include <QTemporaryDir>
#include <QtTest/QtTest>

#include "renderers/BuiltinGitRenderer.h"

using namespace diffy;

namespace {

void writeFile(const QString& path, const QByteArray& contents) {
  QFile file(path);
  QVERIFY2(file.open(QIODevice::WriteOnly | QIODevice::Truncate), qPrintable(file.errorString()));
  file.write(contents);
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

QString createRepoWithMixedChanges() {
  auto* repoDir = new QTemporaryDir;
  if (!repoDir->isValid()) {
    delete repoDir;
    return {};
  }

  runGit(repoDir->path(), {"init"});
  writeFile(repoDir->filePath("modify.txt"), "one\ntwo\nthree\n");
  writeFile(repoDir->filePath("delete.txt"), "goodbye\n");
  writeFile(repoDir->filePath("rename_me.txt"), "keep me\n");
  writeFile(repoDir->filePath("binary.bin"), QByteArray::fromHex("00010203ff"));
  runGit(repoDir->path(), {"add", "."});
  runGit(repoDir->path(),
         {"-c", "user.name=diffy", "-c", "user.email=diffy@example.com", "commit", "-m", "initial"});

  writeFile(repoDir->filePath("modify.txt"), "one\nTWO\nthree\nfour\n");
  QFile::remove(repoDir->filePath("delete.txt"));
  runGit(repoDir->path(), {"rm", "delete.txt"});
  runGit(repoDir->path(), {"mv", "rename_me.txt", "renamed.txt"});
  writeFile(repoDir->filePath("added.txt"), "brand new\n");
  writeFile(repoDir->filePath("binary.bin"), QByteArray::fromHex("00010203ffaa"));
  runGit(repoDir->path(), {"add", "."});
  runGit(repoDir->path(),
         {"-c", "user.name=diffy", "-c", "user.email=diffy@example.com", "commit", "-m", "update"});

  return repoDir->path();
}

const FileDiff* findFile(const QVector<FileDiff>& files, const QString& path) {
  for (const FileDiff& file : files) {
    if (file.path == path) {
      return &file;
    }
  }
  return nullptr;
}

}  // namespace

class BuiltinGitRendererTest : public QObject {
  Q_OBJECT

 private slots:
  void rendersMixedRepositoryChangesWithLibgit2() {
    const QString repoPath = createRepoWithMixedChanges();
    QVERIFY(!repoPath.isEmpty());

    BuiltinGitRenderer renderer(nullptr);
    DiffDocument document;
    QString error;

    QVERIFY2(renderer.render(RenderRequest{repoPath, "HEAD~1", "HEAD"}, &document, &error), qPrintable(error));
    QCOMPARE(document.files.size(), 5);

    const FileDiff* modified = findFile(document.files, "modify.txt");
    QVERIFY(modified != nullptr);
    QCOMPARE(modified->status, QString("M"));
    QCOMPARE(modified->additions, 2);
    QCOMPARE(modified->deletions, 1);
    QVERIFY(!modified->hunks.isEmpty());

    const FileDiff* added = findFile(document.files, "added.txt");
    QVERIFY(added != nullptr);
    QCOMPARE(added->status, QString("A"));

    const FileDiff* deleted = findFile(document.files, "delete.txt");
    QVERIFY(deleted != nullptr);
    QCOMPARE(deleted->status, QString("D"));

    const FileDiff* renamed = findFile(document.files, "renamed.txt");
    QVERIFY(renamed != nullptr);
    QCOMPARE(renamed->status, QString("R"));

    const FileDiff* binary = findFile(document.files, "binary.bin");
    QVERIFY(binary != nullptr);
    QVERIFY(binary->isBinary);
  }
};

QTEST_MAIN(BuiltinGitRendererTest)
#include "test_builtin_git_renderer.moc"
