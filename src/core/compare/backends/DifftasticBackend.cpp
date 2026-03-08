#include "core/compare/backends/DifftasticBackend.h"

#include <git2.h>
#include <simdjson.h>

#include <QByteArray>
#include <QFile>
#include <QProcess>
#include <QStandardPaths>
#include <QTemporaryDir>
#include <QString>
#include <QVector>

namespace diffy {
namespace {

using simdjson::dom::array;
using simdjson::dom::element;
using simdjson::dom::object;

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

std::string mapFileStatus(std::string_view status, std::string_view fallbackStatus) {
  if (status == "created") {
    return "A";
  }
  if (status == "deleted") {
    return "D";
  }
  if (status == "unchanged") {
    return "U";
  }
  return std::string(fallbackStatus);
}

std::string getStringOrDefault(const object& jsonObject, std::string_view key, std::string_view fallback = {}) {
  std::string_view value;
  if (jsonObject.at_key(key).get_string().get(value) == simdjson::SUCCESS) {
    return std::string(value);
  }
  return std::string(fallback);
}

bool getObjectField(const object& jsonObject, std::string_view key, object& value) {
  return jsonObject.at_key(key).get_object().get(value) == simdjson::SUCCESS;
}

bool getArrayField(const object& jsonObject, std::string_view key, array& value) {
  return jsonObject.at_key(key).get_array().get(value) == simdjson::SUCCESS;
}

std::string extractSideText(const object& side, std::vector<TokenSpan>* outTokens) {
  array changes;
  std::string text;
  if (getArrayField(side, "changes", changes)) {
    for (element changeValue : changes) {
      object change;
      if (changeValue.get_object().get(change) != simdjson::SUCCESS) {
        continue;
      }

      const std::string part = getStringOrDefault(change, "content");
      if (part.empty()) {
        continue;
      }
      const int start = static_cast<int>(text.size());
      text += part;
      if (outTokens != nullptr) {
        outTokens->push_back(TokenSpan{start, static_cast<int>(part.size())});
      }
    }
  }

  if (text.empty()) {
    text = getStringOrDefault(side, "text");
  }
  return text;
}

int lineNumberFromSide(const object& side) {
  int64_t lineNumber = -1;
  if (side.at_key("line_number").get_int64().get(lineNumber) != simdjson::SUCCESS) {
    return -1;
  }
  return static_cast<int>(lineNumber) + 1;
}

bool firstFileObject(const element& root, object& fileObject) {
  if (root.get_object().get(fileObject) == simdjson::SUCCESS) {
    return true;
  }

  array files;
  if (root.get_array().get(files) != simdjson::SUCCESS) {
    return false;
  }
  for (element fileValue : files) {
    if (fileValue.get_object().get(fileObject) == simdjson::SUCCESS) {
      return true;
    }
  }
  return false;
}

}  // namespace

bool DifftasticBackend::isAvailable() {
  return !QStandardPaths::findExecutable("difft").isEmpty();
}

std::string_view DifftasticBackend::id() const {
  return "difftastic";
}

bool DifftasticBackend::compare(const CompareRequest& request, DiffDocument* out, std::string* error) const {
  if (!isAvailable()) {
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

    const QByteArray json = difft.readAllStandardOutput();
    FileDiff fileDiff;
    if (!parseDifftasticJson(std::string_view(json.constData(), static_cast<size_t>(json.size())),
                             changed.newPath.toStdString(), changed.status.toStdString(), &fileDiff, error)) {
      return false;
    }
    if (fileDiff.path.empty()) {
      fileDiff.path = changed.newPath.toStdString();
    }
    if (fileDiff.status.empty()) {
      fileDiff.status = changed.status.toStdString();
    }

    doc.files.push_back(std::move(fileDiff));
    ++index;
  }

  cleanup();
  *out = std::move(doc);
  return true;
}

bool DifftasticBackend::parseDifftasticJson(std::string_view json,
                                            std::string_view fallbackPath,
                                            std::string_view fallbackStatus,
                                            FileDiff* outFile,
                                            std::string* error) const {
  simdjson::dom::parser parser;
  simdjson::padded_string padded(json);
  element root;
  if (const auto parseError = parser.parse(padded).get(root); parseError != simdjson::SUCCESS) {
    if (error) {
      *error = "Failed to parse difftastic JSON: " + std::string(simdjson::error_message(parseError));
    }
    return false;
  }

  object fileObject;
  if (!firstFileObject(root, fileObject)) {
    if (error) {
      *error = "difftastic JSON payload did not include a file object";
    }
    return false;
  }

  FileDiff file;
  file.path = getStringOrDefault(fileObject, "path", fallbackPath);
  file.status = mapFileStatus(getStringOrDefault(fileObject, "status"), fallbackStatus);

  if (getStringOrDefault(fileObject, "language") == "binary") {
    file.isBinary = true;
    *outFile = std::move(file);
    return true;
  }

  array chunks;
  if (!getArrayField(fileObject, "chunks", chunks)) {
    *outFile = std::move(file);
    return true;
  }

  for (element chunkValue : chunks) {
    array lines;
    if (chunkValue.get_array().get(lines) != simdjson::SUCCESS) {
      continue;
    }

    Hunk hunk;
    hunk.header = "@@";

    for (element lineValue : lines) {
      object lineObject;
      if (lineValue.get_object().get(lineObject) != simdjson::SUCCESS) {
        continue;
      }

      object lhs;
      object rhs;
      const bool hasLhsObject = getObjectField(lineObject, "lhs", lhs);
      const bool hasRhsObject = getObjectField(lineObject, "rhs", rhs);

      std::vector<TokenSpan> lhsTokens;
      std::vector<TokenSpan> rhsTokens;
      const std::string lhsText = hasLhsObject ? extractSideText(lhs, &lhsTokens) : std::string{};
      const std::string rhsText = hasRhsObject ? extractSideText(rhs, &rhsTokens) : std::string{};
      const int lhsLine = hasLhsObject ? lineNumberFromSide(lhs) : -1;
      const int rhsLine = hasRhsObject ? lineNumberFromSide(rhs) : -1;

      const bool hasLhs = hasLhsObject && (!lhsText.empty() || lhsLine > 0);
      const bool hasRhs = hasRhsObject && (!rhsText.empty() || rhsLine > 0);

      if (hasLhs && hasRhs && lhsText == rhsText) {
        hunk.lines.push_back(DiffLine{lhsLine, rhsLine, LineKind::Context, lhsText, {}});
        continue;
      }

      if (hasLhs) {
        hunk.lines.push_back(DiffLine{lhsLine, -1, LineKind::Deletion, lhsText, std::move(lhsTokens)});
        file.deletions += 1;
      }
      if (hasRhs) {
        hunk.lines.push_back(DiffLine{-1, rhsLine, LineKind::Addition, rhsText, std::move(rhsTokens)});
        file.additions += 1;
      }
    }

    if (!hunk.lines.empty()) {
      file.hunks.push_back(std::move(hunk));
    }
  }

  *outFile = std::move(file);
  return true;
}

}  // namespace diffy
