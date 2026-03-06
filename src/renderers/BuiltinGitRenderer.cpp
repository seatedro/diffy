#include "renderers/BuiltinGitRenderer.h"

#include <git2.h>

namespace diffy {
namespace {

std::string toUtf8(const QString& value) {
  const QByteArray utf8 = value.toUtf8();
  return std::string(utf8.constData(), static_cast<size_t>(utf8.size()));
}

QString lastGitError(const QString& fallback) {
  if (const git_error* err = git_error_last(); err && err->message) {
    return QString::fromUtf8(err->message);
  }
  return fallback;
}

std::string mapDeltaStatus(git_delta_t status) {
  switch (status) {
    case GIT_DELTA_ADDED:
      return "A";
    case GIT_DELTA_DELETED:
      return "D";
    case GIT_DELTA_RENAMED:
      return "R";
    default:
      return "M";
  }
}

std::string normalizePatchText(const git_diff_line* line) {
  QByteArray content(line->content, static_cast<int>(line->content_len));
  if (content.endsWith('\n')) {
    content.chop(1);
  }
  if (content.endsWith('\r')) {
    content.chop(1);
  }
  return std::string(content.constData(), static_cast<size_t>(content.size()));
}

std::vector<TokenSpan> fullLineTokens(const std::string& text) {
  if (text.empty()) {
    return {};
  }
  return std::vector<TokenSpan>{TokenSpan{0, static_cast<int>(text.size())}};
}

bool lookupCommit(git_repository* repo, const QString& revision, git_commit** outCommit, QString* error) {
  git_object* object = nullptr;
  git_object* peeled = nullptr;

  const QByteArray revUtf8 = revision.toUtf8();
  if (git_revparse_single(&object, repo, revUtf8.constData()) != 0) {
    if (error) {
      *error = lastGitError(QString("Failed to resolve revision: %1").arg(revision));
    }
    return false;
  }

  if (git_object_peel(&peeled, object, GIT_OBJECT_COMMIT) != 0) {
    git_object_free(object);
    if (error) {
      *error = lastGitError(QString("Revision is not a commit: %1").arg(revision));
    }
    return false;
  }

  *outCommit = reinterpret_cast<git_commit*>(peeled);
  git_object_free(object);
  return true;
}

std::string pathForDelta(const git_diff_delta* delta) {
  if (delta->status == GIT_DELTA_DELETED && delta->old_file.path != nullptr) {
    return delta->old_file.path;
  }
  if (delta->new_file.path != nullptr) {
    return delta->new_file.path;
  }
  if (delta->old_file.path != nullptr) {
    return delta->old_file.path;
  }
  return "unknown";
}

}  // namespace

BuiltinGitRenderer::BuiltinGitRenderer(const UnifiedDiffParser* parser) : parser_(parser) {}

QString BuiltinGitRenderer::id() const {
  return "builtin";
}

bool BuiltinGitRenderer::render(const RenderRequest& request, DiffDocument* out, QString* error) {
  git_libgit2_init();
  git_repository* repo = nullptr;
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
    git_repository_free(repo);
    git_libgit2_shutdown();
  };

  if (git_repository_open_ext(&repo, request.repoPath.toUtf8().constData(), 0, nullptr) != 0) {
    if (error) {
      *error = lastGitError(QString("Failed to open repository: %1").arg(request.repoPath));
    }
    cleanup();
    return false;
  }

  if (!lookupCommit(repo, request.leftRevision, &leftCommit, error) ||
      !lookupCommit(repo, request.rightRevision, &rightCommit, error)) {
    cleanup();
    return false;
  }

  if (git_commit_tree(&leftTree, leftCommit) != 0 || git_commit_tree(&rightTree, rightCommit) != 0) {
    if (error) {
      *error = lastGitError("Failed to load commit trees");
    }
    cleanup();
    return false;
  }

  git_diff_options diffOptions = GIT_DIFF_OPTIONS_INIT;
  diffOptions.context_lines = 3;
  if (git_diff_tree_to_tree(&diff, repo, leftTree, rightTree, &diffOptions) != 0) {
    if (error) {
      *error = lastGitError("Failed to compute repository diff");
    }
    cleanup();
    return false;
  }

  git_diff_find_options findOptions = GIT_DIFF_FIND_OPTIONS_INIT;
  findOptions.flags = GIT_DIFF_FIND_RENAMES;
  git_diff_find_similar(diff, &findOptions);

  DiffDocument document;
  document.leftRevision = toUtf8(request.leftRevision);
  document.rightRevision = toUtf8(request.rightRevision);

  const size_t deltaCount = git_diff_num_deltas(diff);
  for (size_t deltaIndex = 0; deltaIndex < deltaCount; ++deltaIndex) {
    const git_diff_delta* delta = git_diff_get_delta(diff, deltaIndex);
    if (delta == nullptr) {
      continue;
    }

    FileDiff file;
    file.path = pathForDelta(delta);
    file.status = mapDeltaStatus(delta->status);

    git_patch* patch = nullptr;
    if (git_patch_from_diff(&patch, diff, deltaIndex) != 0) {
      if (error) {
        *error = lastGitError(QString("Failed to build patch for %1").arg(QString::fromUtf8(file.path)));
      }
      cleanup();
      return false;
    }

    const git_diff_delta* resolvedDelta = git_diff_get_delta(diff, deltaIndex);
    const bool isBinaryFlag = resolvedDelta != nullptr && (resolvedDelta->flags & GIT_DIFF_FLAG_BINARY) != 0;
    file.isBinary = isBinaryFlag || patch == nullptr;

    if (patch == nullptr) {
      document.files.push_back(file);
      continue;
    }

    size_t contextLines = 0;
    size_t additionLines = 0;
    size_t deletionLines = 0;
    git_patch_line_stats(&contextLines, &additionLines, &deletionLines, patch);
    file.additions = static_cast<int>(additionLines);
    file.deletions = static_cast<int>(deletionLines);

    const size_t hunkCount = git_patch_num_hunks(patch);
    for (size_t hunkIndex = 0; hunkIndex < hunkCount; ++hunkIndex) {
      const git_diff_hunk* gitHunk = nullptr;
      size_t lineCount = 0;
      if (git_patch_get_hunk(&gitHunk, &lineCount, patch, hunkIndex) != 0 || gitHunk == nullptr) {
        continue;
      }

      Hunk hunk;
      hunk.header = QString::fromUtf8(gitHunk->header, static_cast<int>(gitHunk->header_len)).trimmed().toStdString();
      hunk.collapsed = false;

      for (size_t lineIndex = 0; lineIndex < lineCount; ++lineIndex) {
        const git_diff_line* gitLine = nullptr;
        if (git_patch_get_line_in_hunk(&gitLine, patch, hunkIndex, lineIndex) != 0 || gitLine == nullptr) {
          continue;
        }

        if (gitLine->origin == GIT_DIFF_LINE_CONTEXT_EOFNL || gitLine->origin == GIT_DIFF_LINE_ADD_EOFNL ||
            gitLine->origin == GIT_DIFF_LINE_DEL_EOFNL || gitLine->origin == GIT_DIFF_LINE_FILE_HDR ||
            gitLine->origin == GIT_DIFF_LINE_HUNK_HDR || gitLine->origin == GIT_DIFF_LINE_BINARY) {
          continue;
        }

        DiffLine line;
        line.text = normalizePatchText(gitLine);
        line.oldLine = gitLine->old_lineno;
        line.newLine = gitLine->new_lineno;

        if (gitLine->origin == GIT_DIFF_LINE_ADDITION) {
          line.kind = LineKind::Addition;
          line.oldLine = -1;
          line.tokens = fullLineTokens(line.text);
        } else if (gitLine->origin == GIT_DIFF_LINE_DELETION) {
          line.kind = LineKind::Deletion;
          line.newLine = -1;
          line.tokens = fullLineTokens(line.text);
        } else {
          line.kind = LineKind::Context;
        }

        hunk.lines.push_back(line);
      }

      file.hunks.push_back(hunk);
    }

    git_patch_free(patch);
    document.files.push_back(file);
  }

  cleanup();
  *out = document;
  return true;
}

}  // namespace diffy
