#include "renderers/DifftasticRenderer.h"

#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QProcess>
#include <QStandardPaths>
#include <QTemporaryDir>

namespace diffy {
namespace {

struct ChangedPath {
  QString status;
  QString oldPath;
  QString newPath;
};

QString mapStatus(const QString& rawStatus) {
  if (rawStatus.startsWith('A')) {
    return "A";
  }
  if (rawStatus.startsWith('D')) {
    return "D";
  }
  if (rawStatus.startsWith('R')) {
    return "R";
  }
  return "M";
}

bool runGit(const QString& repoPath,
            const QStringList& args,
            QByteArray* out,
            QString* error,
            bool allowMissing = false) {
  QProcess process;
  process.setProgram("git");

  QStringList fullArgs{"-C", repoPath};
  fullArgs.append(args);
  process.setArguments(fullArgs);
  process.start();

  if (!process.waitForFinished(120000)) {
    if (error) {
      *error = "Timed out while running git command";
    }
    return false;
  }

  const QByteArray stdoutData = process.readAllStandardOutput();
  const QByteArray stderrData = process.readAllStandardError();

  if (process.exitStatus() != QProcess::NormalExit || process.exitCode() != 0) {
    if (allowMissing) {
      return false;
    }
    if (error) {
      *error = QString("git command failed: %1").arg(QString::fromUtf8(stderrData).trimmed());
    }
    return false;
  }

  if (out != nullptr) {
    *out = stdoutData;
  }
  return true;
}

QString extractSideText(const QJsonObject& side, QVector<TokenSpan>* outTokens) {
  const QJsonArray changes = side.value("changes").toArray();
  QString text;
  for (const QJsonValue& changeValue : changes) {
    const QJsonObject change = changeValue.toObject();
    const QString part = change.value("content").toString();
    if (part.isEmpty()) {
      continue;
    }
    const int start = text.size();
    text += part;
    if (outTokens != nullptr) {
      outTokens->push_back(TokenSpan{start, static_cast<int>(part.size())});
    }
  }

  if (text.isEmpty()) {
    text = side.value("text").toString();
  }

  return text;
}

int lineNumberFromSide(const QJsonObject& side) {
  if (!side.contains("line_number")) {
    return -1;
  }
  // difftastic JSON line numbers are zero-based.
  return side.value("line_number").toInt(-1) + 1;
}

}  // namespace

QString DifftasticRenderer::id() const {
  return "difftastic";
}

bool DifftasticRenderer::render(const RenderRequest& request, DiffDocument* out, QString* error) {
  if (QStandardPaths::findExecutable("difft").isEmpty()) {
    if (error) {
      *error = "difftastic executable `difft` was not found in PATH";
    }
    return false;
  }

  QByteArray changedFilesOutput;
  if (!runGit(request.repoPath,
              {"diff", "--name-status", request.leftRevision, request.rightRevision},
              &changedFilesOutput,
              error)) {
    return false;
  }

  QVector<ChangedPath> changedPaths;
  const QStringList changedLines = QString::fromUtf8(changedFilesOutput).split('\n', Qt::SkipEmptyParts);
  for (const QString& line : changedLines) {
    const QStringList parts = line.split('\t');
    if (parts.isEmpty()) {
      continue;
    }

    ChangedPath path;
    path.status = mapStatus(parts.at(0));

    if (path.status == "R" && parts.size() >= 3) {
      path.oldPath = parts.at(1);
      path.newPath = parts.at(2);
    } else if (parts.size() >= 2) {
      path.oldPath = parts.at(1);
      path.newPath = parts.at(1);
    } else {
      continue;
    }

    changedPaths.push_back(path);
  }

  DiffDocument doc;
  doc.leftRevision = request.leftRevision;
  doc.rightRevision = request.rightRevision;

  QTemporaryDir tempDir;
  if (!tempDir.isValid()) {
    if (error) {
      *error = "Failed to create temporary directory for difftastic rendering";
    }
    return false;
  }

  int index = 0;
  for (const ChangedPath& changed : changedPaths) {
    const QString oldTempPath = QString("%1/old_%2.txt").arg(tempDir.path()).arg(index);
    const QString newTempPath = QString("%1/new_%2.txt").arg(tempDir.path()).arg(index);

    QByteArray oldContent;
    QByteArray newContent;

    if (changed.status != "A") {
      runGit(request.repoPath,
             {"show", QString("%1:%2").arg(request.leftRevision, changed.oldPath)},
             &oldContent,
             nullptr,
             true);
    }

    if (changed.status != "D") {
      runGit(request.repoPath,
             {"show", QString("%1:%2").arg(request.rightRevision, changed.newPath)},
             &newContent,
             nullptr,
             true);
    }

    QFile oldFile(oldTempPath);
    if (!oldFile.open(QIODevice::WriteOnly)) {
      if (error) {
        *error = QString("Failed to write temp file: %1").arg(oldTempPath);
      }
      return false;
    }
    oldFile.write(oldContent);
    oldFile.close();

    QFile newFile(newTempPath);
    if (!newFile.open(QIODevice::WriteOnly)) {
      if (error) {
        *error = QString("Failed to write temp file: %1").arg(newTempPath);
      }
      return false;
    }
    newFile.write(newContent);
    newFile.close();

    QProcess difft;
    difft.setProgram("difft");
    difft.setArguments({"--display", "json", oldTempPath, newTempPath});

    QProcessEnvironment env = QProcessEnvironment::systemEnvironment();
    env.insert("DFT_UNSTABLE", "yes");
    difft.setProcessEnvironment(env);

    difft.start();
    if (!difft.waitForFinished(120000)) {
      if (error) {
        *error = "Timed out while running difftastic";
      }
      return false;
    }

    if (difft.exitStatus() != QProcess::NormalExit || difft.exitCode() != 0) {
      if (error) {
        *error = QString("difftastic failed: %1").arg(QString::fromUtf8(difft.readAllStandardError()).trimmed());
      }
      return false;
    }

    FileDiff fileDiff;
    if (!parseDifftasticJson(difft.readAllStandardOutput(), changed.newPath, changed.status, &fileDiff, error)) {
      return false;
    }
    if (fileDiff.path.isEmpty()) {
      fileDiff.path = changed.newPath;
    }
    if (fileDiff.status.isEmpty()) {
      fileDiff.status = changed.status;
    }

    doc.files.push_back(fileDiff);
    ++index;
  }

  *out = doc;
  return true;
}

bool DifftasticRenderer::parseDifftasticJson(const QByteArray& json,
                                             const QString& fallbackPath,
                                             const QString& fallbackStatus,
                                             FileDiff* outFile,
                                             QString* error) const {
  QJsonParseError parseError;
  const QJsonDocument doc = QJsonDocument::fromJson(json, &parseError);
  if (parseError.error != QJsonParseError::NoError) {
    if (error) {
      *error = QString("Failed to parse difftastic JSON: %1").arg(parseError.errorString());
    }
    return false;
  }

  QJsonObject fileObject;
  if (doc.isObject()) {
    fileObject = doc.object();
  } else if (doc.isArray()) {
    const QJsonArray array = doc.array();
    if (array.isEmpty() || !array.first().isObject()) {
      if (error) {
        *error = "difftastic JSON payload did not include a file object";
      }
      return false;
    }
    fileObject = array.first().toObject();
  } else {
    if (error) {
      *error = "difftastic JSON payload was neither object nor array";
    }
    return false;
  }

  FileDiff file;
  file.path = fileObject.value("path").toString(fallbackPath);

  const QString status = fileObject.value("status").toString();
  if (status == "created") {
    file.status = "A";
  } else if (status == "deleted") {
    file.status = "D";
  } else if (status == "unchanged") {
    file.status = "U";
  } else {
    file.status = fallbackStatus;
  }

  if (fileObject.value("language").toString() == "binary") {
    file.isBinary = true;
    *outFile = file;
    return true;
  }

  const QJsonArray chunks = fileObject.value("chunks").toArray();
  for (const QJsonValue& chunkValue : chunks) {
    if (!chunkValue.isArray()) {
      continue;
    }

    Hunk hunk;
    hunk.header = "@@";

    const QJsonArray lines = chunkValue.toArray();
    for (const QJsonValue& lineValue : lines) {
      if (!lineValue.isObject()) {
        continue;
      }

      const QJsonObject lineObject = lineValue.toObject();
      const QJsonObject lhs = lineObject.value("lhs").toObject();
      const QJsonObject rhs = lineObject.value("rhs").toObject();

      QVector<TokenSpan> lhsTokens;
      QVector<TokenSpan> rhsTokens;
      const QString lhsText = extractSideText(lhs, &lhsTokens);
      const QString rhsText = extractSideText(rhs, &rhsTokens);
      const int lhsLine = lineNumberFromSide(lhs);
      const int rhsLine = lineNumberFromSide(rhs);

      const bool hasLhs = !lhs.isEmpty() && (!lhsText.isEmpty() || lhsLine > 0);
      const bool hasRhs = !rhs.isEmpty() && (!rhsText.isEmpty() || rhsLine > 0);

      if (hasLhs && hasRhs && lhsText == rhsText) {
        hunk.lines.push_back(DiffLine{lhsLine, rhsLine, LineKind::Context, lhsText, {}});
        continue;
      }

      if (hasLhs) {
        hunk.lines.push_back(DiffLine{lhsLine, -1, LineKind::Deletion, lhsText, lhsTokens});
        file.deletions += 1;
      }
      if (hasRhs) {
        hunk.lines.push_back(DiffLine{-1, rhsLine, LineKind::Addition, rhsText, rhsTokens});
        file.additions += 1;
      }
    }

    if (!hunk.lines.isEmpty()) {
      file.hunks.push_back(hunk);
    }
  }

  *outFile = file;
  return true;
}

}  // namespace diffy
