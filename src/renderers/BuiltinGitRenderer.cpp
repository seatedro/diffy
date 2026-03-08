#include "renderers/BuiltinGitRenderer.h"

#include <git2.h>

#include <algorithm>
#include <filesystem>
#include <sstream>

#include "core/diff/WordDiff.h"
#include "core/syntax/Highlighter.h"
#include "core/syntax/LanguageRegistry.h"

namespace diffy {
namespace {

std::string lastGitError(const std::string& fallback) {
  if (const git_error* err = git_error_last(); err && err->message) {
    return err->message;
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
  std::string content(line->content, static_cast<size_t>(line->content_len));
  if (!content.empty() && content.back() == '\n') {
    content.pop_back();
  }
  if (!content.empty() && content.back() == '\r') {
    content.pop_back();
  }
  return content;
}

std::vector<TokenSpan> fullLineTokens(const std::string& text) {
  if (text.empty()) {
    return {};
  }
  return std::vector<TokenSpan>{TokenSpan{0, static_cast<int>(text.size())}};
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

std::string trimAscii(std::string value) {
  while (!value.empty() && std::isspace(static_cast<unsigned char>(value.back())) != 0) {
    value.pop_back();
  }
  size_t start = 0;
  while (start < value.size() && std::isspace(static_cast<unsigned char>(value[start])) != 0) {
    ++start;
  }
  if (start > 0) {
    value.erase(0, start);
  }
  return value;
}

}  // namespace

std::string_view BuiltinGitRenderer::id() const {
  return "builtin";
}

bool BuiltinGitRenderer::render(const RenderRequest& request, DiffDocument* out, std::string* error) {
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

  if (git_repository_open_ext(&repo, request.repoPath.c_str(), 0, nullptr) != 0) {
    if (error) {
      *error = lastGitError("Failed to open repository: " + request.repoPath);
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
  document.leftRevision = request.leftRevision;
  document.rightRevision = request.rightRevision;

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
        *error = lastGitError("Failed to build patch for " + file.path);
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
      hunk.header = trimAscii(std::string(gitHunk->header, static_cast<size_t>(gitHunk->header_len)));
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
          line.changeSpans = fullLineTokens(line.text);
        } else if (gitLine->origin == GIT_DIFF_LINE_DELETION) {
          line.kind = LineKind::Deletion;
          line.newLine = -1;
          line.changeSpans = fullLineTokens(line.text);
        } else {
          line.kind = LineKind::Context;
        }

        hunk.lines.push_back(line);
      }

      for (size_t lineIdx = 0; lineIdx < hunk.lines.size();) {
        if (hunk.lines[lineIdx].kind != LineKind::Deletion) {
          ++lineIdx;
          continue;
        }
        size_t delStart = lineIdx;
        while (lineIdx < hunk.lines.size() && hunk.lines[lineIdx].kind == LineKind::Deletion) {
          ++lineIdx;
        }
        size_t addStart = lineIdx;
        while (lineIdx < hunk.lines.size() && hunk.lines[lineIdx].kind == LineKind::Addition) {
          ++lineIdx;
        }
        size_t pairCount = std::min(lineIdx - addStart, addStart - delStart);
        for (size_t pairIdx = 0; pairIdx < pairCount; ++pairIdx) {
          DiffLine& del = hunk.lines[delStart + pairIdx];
          DiffLine& add = hunk.lines[addStart + pairIdx];
          auto wordResult = computeWordDiff(del.text, add.text);
          if (!wordResult.leftTokens.empty() || !wordResult.rightTokens.empty()) {
            del.changeSpans = std::move(wordResult.leftTokens);
            add.changeSpans = std::move(wordResult.rightTokens);
          }
        }
      }

      file.hunks.push_back(hunk);
    }

    git_patch_free(patch);
    document.files.push_back(file);
  }

  cleanup();

  applySyntaxHighlighting(document);

  *out = document;
  return true;
}

void BuiltinGitRenderer::setSyntax(const LanguageRegistry* registry, const Highlighter* highlighter) {
  languageRegistry_ = registry;
  highlighter_ = highlighter;
}

void BuiltinGitRenderer::applySyntaxHighlighting(DiffDocument& document) const {
  if (languageRegistry_ == nullptr || highlighter_ == nullptr) {
    return;
  }

  for (FileDiff& file : document.files) {
    if (file.isBinary) {
      continue;
    }

    std::string ext;
    if (auto dotPos = file.path.rfind('.'); dotPos != std::string::npos) {
      ext = file.path.substr(dotPos);
    }
    if (ext.empty()) {
      continue;
    }

    const GrammarInfo* grammar = languageRegistry_->grammarForExtension(ext);
    if (grammar == nullptr) {
      continue;
    }

    std::string oldContent;
    std::string newContent;
    struct LineRef {
      size_t hunkIdx;
      size_t lineIdx;
      size_t contentOffset;
      size_t contentLen;
    };
    std::vector<LineRef> oldLineRefs;
    std::vector<LineRef> newLineRefs;

    for (size_t hi = 0; hi < file.hunks.size(); ++hi) {
      const Hunk& hunk = file.hunks[hi];
      for (size_t li = 0; li < hunk.lines.size(); ++li) {
        const DiffLine& line = hunk.lines[li];
        if (line.kind == LineKind::Deletion || line.kind == LineKind::Context) {
          size_t offset = oldContent.size();
          oldContent += line.text;
          oldContent += '\n';
          oldLineRefs.push_back({hi, li, offset, line.text.size()});
        }
        if (line.kind == LineKind::Addition || line.kind == LineKind::Context) {
          size_t offset = newContent.size();
          newContent += line.text;
          newContent += '\n';
          newLineRefs.push_back({hi, li, offset, line.text.size()});
        }
      }
    }

    auto oldTokens = highlighter_->highlight(*grammar, oldContent);
    auto newTokens = highlighter_->highlight(*grammar, newContent);

    auto distributeTokens = [&](const std::vector<TokenSpan>& tokens,
                                const std::vector<LineRef>& lineRefs, bool isNew) {
      size_t tokenIdx = 0;
      for (const LineRef& ref : lineRefs) {
        const int lineStart = static_cast<int>(ref.contentOffset);
        const int lineEnd = lineStart + static_cast<int>(ref.contentLen);

        while (tokenIdx < tokens.size() && tokens[tokenIdx].start + tokens[tokenIdx].length <= lineStart) {
          ++tokenIdx;
        }

        DiffLine& line = file.hunks[ref.hunkIdx].lines[ref.lineIdx];

        std::vector<TokenSpan> syntaxTokens;
        for (size_t si = tokenIdx; si < tokens.size(); ++si) {
          const TokenSpan& tok = tokens[si];
          if (tok.start >= lineEnd) {
            break;
          }
          int clampedStart = std::max(tok.start, lineStart);
          int clampedEnd = std::min(tok.start + tok.length, lineEnd);
          if (clampedEnd > clampedStart) {
            syntaxTokens.push_back(TokenSpan{
                clampedStart - lineStart,
                clampedEnd - clampedStart,
                tok.syntaxKind,
            });
          }
        }

        if (!syntaxTokens.empty()) {
          line.tokens = std::move(syntaxTokens);
        }
      }
    };

    distributeTokens(oldTokens, oldLineRefs, false);
    distributeTokens(newTokens, newLineRefs, true);
  }
}

}  // namespace diffy
