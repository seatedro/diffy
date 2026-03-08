#pragma once

#include <string_view>
#include <vector>

#include "core/diff/DiffTypes.h"
#include "core/syntax/LanguageRegistry.h"

namespace diffy {

class Highlighter {
 public:
  Highlighter();
  ~Highlighter();

  Highlighter(const Highlighter&) = delete;
  Highlighter& operator=(const Highlighter&) = delete;

  std::vector<TokenSpan> highlight(const GrammarInfo& grammar, std::string_view source) const;

 private:
  struct Impl;
  Impl* impl_;
};

}  // namespace diffy
