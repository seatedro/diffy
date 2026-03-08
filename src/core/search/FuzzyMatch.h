#pragma once

#include <string>
#include <string_view>
#include <vector>

namespace diffy {

struct FuzzyResult {
  int index = -1;
  int score = 0;
};

int fuzzyScore(std::string_view query, std::string_view candidate);

std::vector<FuzzyResult> fuzzyRank(std::string_view query,
                                    const std::vector<std::string>& candidates,
                                    int maxResults = 50);

}  // namespace diffy
