#include "core/syntax/DiffSyntaxAnnotator.h"

#include "core/syntax/Highlighter.h"
#include "core/syntax/LanguageRegistry.h"

namespace diffy {

void DiffSyntaxAnnotator::annotateFile(const LanguageRegistry& registry,
                                       const Highlighter& highlighter,
                                       FileDiff& file) const {
  if (file.isBinary) {
    return;
  }

  std::string ext;
  if (const auto dotPos = file.path.rfind('.'); dotPos != std::string::npos) {
    ext = file.path.substr(dotPos);
  }
  if (ext.empty()) {
    return;
  }

  const GrammarInfo* grammar = registry.grammarForExtension(ext);
  if (grammar == nullptr) {
    return;
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

  size_t totalLines = 0;
  for (const Hunk& hunk : file.hunks) {
    totalLines += hunk.lines.size();
  }
  oldLineRefs.reserve(totalLines);
  newLineRefs.reserve(totalLines);
  oldContent.reserve(totalLines * 40);
  newContent.reserve(totalLines * 40);

  for (size_t hunkIndex = 0; hunkIndex < file.hunks.size(); ++hunkIndex) {
    const Hunk& hunk = file.hunks[hunkIndex];
    for (size_t lineIndex = 0; lineIndex < hunk.lines.size(); ++lineIndex) {
      const DiffLine& line = hunk.lines[lineIndex];
      if (line.kind == LineKind::Deletion || line.kind == LineKind::Context) {
        const size_t offset = oldContent.size();
        oldContent += line.text;
        oldContent += '\n';
        oldLineRefs.push_back({hunkIndex, lineIndex, offset, line.text.size()});
      }
      if (line.kind == LineKind::Addition || line.kind == LineKind::Context) {
        const size_t offset = newContent.size();
        newContent += line.text;
        newContent += '\n';
        newLineRefs.push_back({hunkIndex, lineIndex, offset, line.text.size()});
      }
    }
  }

  const auto oldTokens = highlighter.highlight(*grammar, oldContent);
  const auto newTokens = highlighter.highlight(*grammar, newContent);

  auto distributeTokens = [&](const std::vector<TokenSpan>& tokens, const std::vector<LineRef>& lineRefs) {
    size_t tokenIndex = 0;
    for (const LineRef& ref : lineRefs) {
      const int lineStart = static_cast<int>(ref.contentOffset);
      const int lineEnd = lineStart + static_cast<int>(ref.contentLen);

      while (tokenIndex < tokens.size() && tokens[tokenIndex].start + tokens[tokenIndex].length <= lineStart) {
        ++tokenIndex;
      }

      DiffLine& line = file.hunks[ref.hunkIdx].lines[ref.lineIdx];
      std::vector<TokenSpan> syntaxTokens;
      for (size_t index = tokenIndex; index < tokens.size(); ++index) {
        const TokenSpan& token = tokens[index];
        if (token.start >= lineEnd) {
          break;
        }
        const int start = std::max(lineStart, token.start);
        const int end = std::min(lineEnd, token.start + token.length);
        if (end <= start) {
          continue;
        }
        syntaxTokens.push_back(TokenSpan{start - lineStart, end - start, token.syntaxKind});
      }
      if (!syntaxTokens.empty()) {
        line.tokens = std::move(syntaxTokens);
      }
    }
  };

  distributeTokens(oldTokens, oldLineRefs);
  distributeTokens(newTokens, newLineRefs);
}

void DiffSyntaxAnnotator::annotateFiles(const LanguageRegistry& registry,
                                        const Highlighter& highlighter,
                                        std::vector<FileDiff>& files,
                                        int skipIndex) const {
  for (size_t index = 0; index < files.size(); ++index) {
    if (static_cast<int>(index) == skipIndex) {
      continue;
    }
    annotateFile(registry, highlighter, files[index]);
  }
}

}  // namespace diffy
