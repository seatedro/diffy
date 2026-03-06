#include "renderers/DifftasticRenderer.h"

#include <git2.h>

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
  QByteArray oldContent;
  QByteArray newContent;
  bool isBinary = false;
};

std::string lastGitError(const std::string& fallback) {
  if (const git_error* err = git_error_last(); err && err->message) {
    return err->message;
  }
  return fallback;
}

QString mapStatus(git_delta_t rawStatus) {
  if (rawStatus == GIT_DELTA_ADDED) {
    return "A";
  }
  if (rawStatus == GIT_DELTA_DELETED) {
    return "D";
  }
  if (rawStatus == GIT_DELTA_RENAMED) {
    return "R";
  }
  return "M";
}

bool lookupCommit(git_repository* repo, const std::string& revision, git_commit** outCommit, std::string* error) {
  git_object* object = nullptr;
  git_object* peeled = nullptr;

  if (git_revparse_single(&object, repo, revision.c_str()) != 0) {
    if (error) {
      *error = lastGitError("Failed to resolve revision: " + revision);
    }
    return false;
  }

  if (git_object_peel(&peeled, object, GIT_OBJECT_COMMIT) != 0) {
    git_object_free(object);
    if (error) {
      *error = lastGitError("Revision is not a commit: " + revision);
    }
    return false;
  }

  *outCommit = reinterpret_cast<git_commit*>(peeled);
  git_object_free(object);
  return true;
}

QString diffPath(const git_diff_delta* delta, bool useOldPath = false) {
  if (useOldPath && delta->old_file.path != nullptr) {
    return QString::fromUtf8(delta->old_file.path);
  }
  if (delta->new_file.path != nullptr) {
    return QString::fromUtf8(delta->new_file.path);
  }
  if (delta->old_file.path != nullptr) {
    return QString::fromUtf8(delta->old_file.path);
  }
  return "unknown";
}

bool loadBlobContent(git_repository* repo,
                     const git_oid& oid,
                     bool present,
                     QByteArray* outContent,
                     bool* outIsBinary,
                     std::string* error) {
  if (!present) {
    if (outContent != nullptr) {
      outContent->clear();
    }
    if (outIsBinary != nullptr) {
      *outIsBinary = false;
    }
    return true;
  }

  git_blob* blob = nullptr;
  if (git_blob_lookup(&blob, repo, &oid) != 0) {
    if (error) {
      *error = lastGitError(std::string("Failed to load blob for difftastic rendering"));
    }
    return false;
  }

  if (outIsBinary != nullptr) {
    *outIsBinary = git_blob_is_binary(blob) != 0;
  }
  if (outContent != nullptr) {
    outContent->setRawData(static_cast<const char*>(git_blob_rawcontent(blob)),
                           static_cast<int>(git_blob_rawsize(blob)));
    *outContent = QByteArray(outContent->constData(), outContent->size());
  }

  git_blob_free(blob);
  return true;
}

bool collectChangedPaths(git_repository* repo,
                         const std::string& leftRevision,
                         const std::string& rightRevision,
                         QVector<ChangedPath>* outPaths,
                         std::string* error) {
  git_commit* leftCommit = nullptr;
  git_commit* rightCommit = nullptr;
  git_tree* leftTree = nullptr;
  git_tree* rightTree = nullptr;
  git_diff* diff = nullptr;

  auto cleanup = [&]() {
    git_diff_free(diff);
    git_tree_free(leftTree);
    git_tree_free(rightTree);
    git_commit_free(leftCommit);
    git_commit_free(rightCommit);
  };

  if (!lookupCommit(repo, leftRevision, &leftCommit, error) ||
      !lookupCommit(repo, rightRevision, &rightCommit, error)) {
    cleanup();
    return false;
  }

  if (git_commit_tree(&leftTree, leftCommit) != 0 || git_commit_tree(&rightTree, rightCommit) != 0) {
    if (error) {
      *error = lastGitError(std::string("Failed to load commit trees"));
    }
    cleanup();
    return false;
  }

  git_diff_options diffOptions = GIT_DIFF_OPTIONS_INIT;
  diffOptions.context_lines = 3;
  if (git_diff_tree_to_tree(&diff, repo, leftTree, rightTree, &diffOptions) != 0) {
    if (error) {
      *error = lastGitError(std::string("Failed to compute repository diff"));
    }
    cleanup();
    return false;
  }

  git_diff_find_options findOptions = GIT_DIFF_FIND_OPTIONS_INIT;
  findOptions.flags = GIT_DIFF_FIND_RENAMES;
  git_diff_find_similar(diff, &findOptions);

  const size_t deltaCount = git_diff_num_deltas(diff);
  outPaths->clear();
  for (size_t deltaIndex = 0; deltaIndex < deltaCount; ++deltaIndex) {
    const git_diff_delta* delta = git_diff_get_delta(diff, deltaIndex);
    if (delta == nullptr) {
      continue;
    }

    ChangedPath path;
    path.status = mapStatus(delta->status);
    path.oldPath = diffPath(delta, true);
    path.newPath = diffPath(delta, false);
    path.isBinary = (delta->flags & GIT_DIFF_FLAG_BINARY) != 0;

    const bool hasOldBlob = delta->status != GIT_DELTA_ADDED && !git_oid_is_zero(&delta->old_file.id);
    const bool hasNewBlob = delta->status != GIT_DELTA_DELETED && !git_oid_is_zero(&delta->new_file.id);
    bool oldBinary = false;
    bool newBinary = false;
    if (!loadBlobContent(repo, delta->old_file.id, hasOldBlob, &path.oldContent, &oldBinary, error) ||
        !loadBlobContent(repo, delta->new_file.id, hasNewBlob, &path.newContent, &newBinary, error)) {
      cleanup();
      return false;
    }
    path.isBinary = path.isBinary || oldBinary || newBinary;
    outPaths->push_back(std::move(path));
  }

  cleanup();
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

std::vector<TokenSpan> toStdTokens(const QVector<TokenSpan>& tokens) {
  return {tokens.begin(), tokens.end()};
}

int lineNumberFromSide(const QJsonObject& side) {
  if (!side.contains("line_number")) {
    return -1;
  }
  // difftastic JSON line numbers are zero-based.
  return side.value("line_number").toInt(-1) + 1;
}

}  // namespace

std::string_view DifftasticRenderer::id() const {
  return "difftastic";
}

bool DifftasticRenderer::render(const RenderRequest& request, DiffDocument* out, std::string* error) {
  if (QStandardPaths::findExecutable("difft").isEmpty()) {
    if (error) {
      *error = "difftastic executable `difft` was not found in PATH";
    }
    return false;
  }

  git_libgit2_init();
  git_repository* repo = nullptr;
  auto cleanup = [&]() {
    git_repository_free(repo);
    git_libgit2_shutdown();
  };

  const QString repoPath = QString::fromStdString(request.repoPath);
  if (git_repository_open_ext(&repo, request.repoPath.c_str(), 0, nullptr) != 0) {
    if (error) {
      *error = lastGitError("Failed to open repository: " + request.repoPath);
    }
    cleanup();
    return false;
  }

  QVector<ChangedPath> changedPaths;
  if (!collectChangedPaths(repo, request.leftRevision, request.rightRevision, &changedPaths, error)) {
    cleanup();
    return false;
  }

  DiffDocument doc;
  doc.leftRevision = request.leftRevision;
  doc.rightRevision = request.rightRevision;

  QTemporaryDir tempDir;
    if (!tempDir.isValid()) {
      if (error) {
        *error = "Failed to create temporary directory for difftastic rendering";
      }
      cleanup();
      return false;
  }

  int index = 0;
  for (const ChangedPath& changed : changedPaths) {
    if (changed.isBinary) {
      FileDiff fileDiff;
      fileDiff.path = changed.newPath.toStdString();
      fileDiff.status = changed.status.toStdString();
      fileDiff.isBinary = true;
      doc.files.push_back(std::move(fileDiff));
      ++index;
      continue;
    }

    const QString oldTempPath = QString("%1/old_%2.txt").arg(tempDir.path()).arg(index);
    const QString newTempPath = QString("%1/new_%2.txt").arg(tempDir.path()).arg(index);

    QFile oldFile(oldTempPath);
    if (!oldFile.open(QIODevice::WriteOnly)) {
      if (error) {
        *error = QString("Failed to write temp file: %1").arg(oldTempPath).toStdString();
      }
      cleanup();
      return false;
    }
    oldFile.write(changed.oldContent);
    oldFile.close();

    QFile newFile(newTempPath);
    if (!newFile.open(QIODevice::WriteOnly)) {
      if (error) {
        *error = QString("Failed to write temp file: %1").arg(newTempPath).toStdString();
      }
      cleanup();
      return false;
    }
    newFile.write(changed.newContent);
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
      cleanup();
      return false;
    }

    if (difft.exitStatus() != QProcess::NormalExit || difft.exitCode() != 0) {
      if (error) {
        *error =
            QString("difftastic failed: %1").arg(QString::fromUtf8(difft.readAllStandardError()).trimmed()).toStdString();
      }
      cleanup();
      return false;
    }

    FileDiff fileDiff;
    if (!parseDifftasticJson(difft.readAllStandardOutput(), changed.newPath, changed.status, &fileDiff, error)) {
      return false;
    }
    if (fileDiff.path.empty()) {
      fileDiff.path = changed.newPath.toStdString();
    }
    if (fileDiff.status.empty()) {
      fileDiff.status = changed.status.toStdString();
    }

    doc.files.push_back(fileDiff);
    ++index;
  }

  cleanup();
  *out = doc;
  return true;
}

bool DifftasticRenderer::parseDifftasticJson(const QByteArray& json,
                                             const QString& fallbackPath,
                                             const QString& fallbackStatus,
                                             FileDiff* outFile,
                                             std::string* error) const {
  QJsonParseError parseError;
  const QJsonDocument doc = QJsonDocument::fromJson(json, &parseError);
  if (parseError.error != QJsonParseError::NoError) {
    if (error) {
      *error = QString("Failed to parse difftastic JSON: %1").arg(parseError.errorString()).toStdString();
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
  file.path = fileObject.value("path").toString(fallbackPath).toStdString();

  const QString status = fileObject.value("status").toString();
  if (status == "created") {
    file.status = "A";
  } else if (status == "deleted") {
    file.status = "D";
  } else if (status == "unchanged") {
    file.status = "U";
  } else {
    file.status = fallbackStatus.toStdString();
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
        hunk.lines.push_back(DiffLine{lhsLine, rhsLine, LineKind::Context, lhsText.toStdString(), {}});
        continue;
      }

      if (hasLhs) {
        hunk.lines.push_back(
            DiffLine{lhsLine, -1, LineKind::Deletion, lhsText.toStdString(), toStdTokens(lhsTokens)});
        file.deletions += 1;
      }
      if (hasRhs) {
        hunk.lines.push_back(
            DiffLine{-1, rhsLine, LineKind::Addition, rhsText.toStdString(), toStdTokens(rhsTokens)});
        file.additions += 1;
      }
    }

    if (!hunk.lines.empty()) {
      file.hunks.push_back(hunk);
    }
  }

  *outFile = file;
  return true;
}

}  // namespace diffy
