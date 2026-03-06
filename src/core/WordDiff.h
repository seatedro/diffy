#pragma once

#include <string_view>
#include <vector>

#include "core/DiffTypes.h"

namespace diffy {

struct WordDiffResult {
  std::vector<TokenSpan> leftTokens;
  std::vector<TokenSpan> rightTokens;
};

WordDiffResult computeWordDiff(std::string_view leftText, std::string_view rightText);

}  // namespace diffy
