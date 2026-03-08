#pragma once

#include <vector>

#include "core/diff/DiffTypes.h"

namespace diffy {

class Highlighter;
class LanguageRegistry;

class DiffSyntaxAnnotator {
 public:
  void annotateFile(const LanguageRegistry& registry, const Highlighter& highlighter, FileDiff& file) const;
  void annotateFiles(const LanguageRegistry& registry,
                     const Highlighter& highlighter,
                     std::vector<FileDiff>& files,
                     int skipIndex = -1) const;
};

}  // namespace diffy
