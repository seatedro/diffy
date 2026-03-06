#include <QBuffer>
#include <QFile>
#include <QHash>
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

QByteArray runGitCapture(const QString& repoPath, const QStringList& arguments) {
  QProcess process;
  process.setProgram("git");
  process.setWorkingDirectory(repoPath);
  process.setArguments(arguments);
  process.start();
  if (!process.waitForFinished(30000)) {
    process.kill();
    process.waitForFinished();
    return {};
  }
  const QString stderrText = QString::fromUtf8(process.readAllStandardError()).trimmed();
  if (process.exitCode() != 0 || process.exitStatus() != QProcess::NormalExit) {
    qWarning().noquote() << stderrText;
    return {};
  }
  return process.readAllStandardOutput();
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

const FileDiff* findFile(const std::vector<FileDiff>& files, std::string_view path) {
  for (const FileDiff& file : files) {
    if (file.path == path) {
      return &file;
    }
  }
  return nullptr;
}

QString normalizeGitPathField(const QString& raw) {
  QString path = raw.trimmed();
  if (path.contains("=>")) {
    path = path.section("=>", 1).trimmed();
    path.remove('{');
    path.remove('}');
  }
  return path;
}

QHash<QString, QString> parseNameStatus(const QByteArray& output) {
  QHash<QString, QString> result;
  const QStringList lines = QString::fromUtf8(output).split('\n', Qt::SkipEmptyParts);
  for (const QString& line : lines) {
    const QStringList parts = line.split('\t');
    if (parts.isEmpty()) {
      continue;
    }

    const QString status = parts.first();
    if (status.startsWith('R') && parts.size() >= 3) {
      result.insert(parts.at(2), "R");
    } else if (status.startsWith('A') && parts.size() >= 2) {
      result.insert(parts.at(1), "A");
    } else if (status.startsWith('D') && parts.size() >= 2) {
      result.insert(parts.at(1), "D");
    } else if (parts.size() >= 2) {
      result.insert(parts.at(1), "M");
    }
  }
  return result;
}

QHash<QString, QPair<int, int>> parseNumstat(const QByteArray& output) {
  QHash<QString, QPair<int, int>> result;
  const QStringList lines = QString::fromUtf8(output).split('\n', Qt::SkipEmptyParts);
  for (const QString& line : lines) {
    const QStringList parts = line.split('\t');
    if (parts.size() < 3) {
      continue;
    }

    const QString path = normalizeGitPathField(parts.last());
    const int additions = parts.at(0) == "-" ? -1 : parts.at(0).toInt();
    const int deletions = parts.at(1) == "-" ? -1 : parts.at(1).toInt();
    result.insert(path, qMakePair(additions, deletions));
  }
  return result;
}

}  // namespace

class BuiltinGitRendererTest : public QObject {
  Q_OBJECT

 private slots:
  void rendersMixedRepositoryChangesWithLibgit2() {
    const QString repoPath = createRepoWithMixedChanges();
    QVERIFY(!repoPath.isEmpty());

    BuiltinGitRenderer renderer;
    DiffDocument document;
    QString error;

    QVERIFY2(renderer.render(RenderRequest{repoPath.toStdString(), "HEAD~1", "HEAD"}, &document, &error),
             qPrintable(error));
    QCOMPARE(document.files.size(), 5);

    const FileDiff* modified = findFile(document.files, "modify.txt");
    QVERIFY(modified != nullptr);
    QCOMPARE(QString::fromUtf8(modified->status), QString("M"));
    QCOMPARE(modified->additions, 2);
    QCOMPARE(modified->deletions, 1);
    QVERIFY(!modified->hunks.empty());

    const FileDiff* added = findFile(document.files, "added.txt");
    QVERIFY(added != nullptr);
    QCOMPARE(QString::fromUtf8(added->status), QString("A"));

    const FileDiff* deleted = findFile(document.files, "delete.txt");
    QVERIFY(deleted != nullptr);
    QCOMPARE(QString::fromUtf8(deleted->status), QString("D"));

    const FileDiff* renamed = findFile(document.files, "renamed.txt");
    QVERIFY(renamed != nullptr);
    QCOMPARE(QString::fromUtf8(renamed->status), QString("R"));

    const FileDiff* binary = findFile(document.files, "binary.bin");
    QVERIFY(binary != nullptr);
    QVERIFY(binary->isBinary);

    const QByteArray nameStatusOutput = runGitCapture(repoPath, {"diff", "--name-status", "-M", "HEAD~1", "HEAD"});
    const QByteArray numstatOutput = runGitCapture(repoPath, {"diff", "--numstat", "-M", "HEAD~1", "HEAD"});
    QVERIFY(!nameStatusOutput.isEmpty());
    QVERIFY(!numstatOutput.isEmpty());

    const auto gitStatuses = parseNameStatus(nameStatusOutput);
    const auto gitNumstat = parseNumstat(numstatOutput);

    QCOMPARE(gitStatuses.value("modify.txt"), QString::fromUtf8(modified->status));
    QCOMPARE(gitStatuses.value("added.txt"), QString::fromUtf8(added->status));
    QCOMPARE(gitStatuses.value("delete.txt"), QString::fromUtf8(deleted->status));
    QCOMPARE(gitStatuses.value("renamed.txt"), QString::fromUtf8(renamed->status));
    QCOMPARE(gitStatuses.value("binary.bin"), QString("M"));

    QCOMPARE(gitNumstat.value("modify.txt").first, modified->additions);
    QCOMPARE(gitNumstat.value("modify.txt").second, modified->deletions);
    QCOMPARE(gitNumstat.value("added.txt").first, added->additions);
    QCOMPARE(gitNumstat.value("added.txt").second, added->deletions);
    QCOMPARE(gitNumstat.value("delete.txt").first, deleted->additions);
    QCOMPARE(gitNumstat.value("delete.txt").second, deleted->deletions);
    QCOMPARE(gitNumstat.value("renamed.txt").first, renamed->additions);
    QCOMPARE(gitNumstat.value("renamed.txt").second, renamed->deletions);
    QCOMPARE(gitNumstat.value("binary.bin").first, -1);
    QCOMPARE(gitNumstat.value("binary.bin").second, -1);
  }
};

QTEST_MAIN(BuiltinGitRendererTest)
#include "test_builtin_git_renderer.moc"
