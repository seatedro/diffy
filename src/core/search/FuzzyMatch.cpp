#include "core/search/FuzzyMatch.h"

#include <algorithm>
#include <cctype>

namespace diffy {
namespace {

constexpr int kNoMatch = 0;
constexpr int kSequentialBonus = 15;
constexpr int kSeparatorBonus = 30;
constexpr int kCamelBonus = 30;
constexpr int kFirstLetterBonus = 15;
constexpr int kLeadingLetterPenalty = -5;
constexpr int kMaxLeadingLetterPenalty = -15;
constexpr int kUnmatchedLetterPenalty = -1;

char toLower(char c) {
  return static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
}

bool isUpper(char c) {
  return std::isupper(static_cast<unsigned char>(c)) != 0;
}

bool isSeparator(char c) {
  return c == '/' || c == '\\' || c == '_' || c == '-' || c == '.' || c == ' ';
}

}  // namespace

int fuzzyScore(std::string_view query, std::string_view candidate) {
  if (query.empty()) return 0;
  if (candidate.empty()) return kNoMatch;

  int score = 0;
  size_t queryIdx = 0;
  bool prevMatched = false;
  bool prevSeparator = true;
  int bestLetterScore = 0;
  bool bestLetterMatched = false;

  for (size_t candIdx = 0; candIdx < candidate.size(); ++candIdx) {
    const char candChar = candidate[candIdx];
    const char candLower = toLower(candChar);
    const bool curSeparator = isSeparator(candChar);

    if (queryIdx < query.size() && toLower(query[queryIdx]) == candLower) {
      int letterScore = 0;

      if (candIdx == 0) {
        letterScore += kFirstLetterBonus;
      }

      if (prevMatched) {
        letterScore += kSequentialBonus;
      }

      if (prevSeparator) {
        letterScore += kSeparatorBonus;
      }

      if (queryIdx > 0 && isUpper(candChar) && !isUpper(candidate[candIdx > 0 ? candIdx - 1 : 0])) {
        letterScore += kCamelBonus;
      }

      score += letterScore;
      ++queryIdx;
      prevMatched = true;

      if (letterScore > bestLetterScore) {
        bestLetterScore = letterScore;
        bestLetterMatched = true;
      }
    } else {
      score += kUnmatchedLetterPenalty;
      prevMatched = false;
    }

    prevSeparator = curSeparator;
  }

  if (queryIdx != query.size()) {
    return kNoMatch;
  }

  // Leading letter penalty (capped)
  size_t firstMatchIdx = 0;
  for (size_t i = 0; i < candidate.size(); ++i) {
    if (toLower(candidate[i]) == toLower(query[0])) {
      firstMatchIdx = i;
      break;
    }
  }
  score += std::max(kMaxLeadingLetterPenalty,
                    kLeadingLetterPenalty * static_cast<int>(firstMatchIdx));

  return score;
}

std::vector<FuzzyResult> fuzzyRank(std::string_view query,
                                    const std::vector<std::string>& candidates,
                                    int maxResults) {
  std::vector<FuzzyResult> results;
  results.reserve(candidates.size());

  for (int i = 0; i < static_cast<int>(candidates.size()); ++i) {
    const int s = fuzzyScore(query, candidates[i]);
    if (s > kNoMatch) {
      results.push_back({i, s});
    }
  }

  std::sort(results.begin(), results.end(), [](const FuzzyResult& a, const FuzzyResult& b) {
    return a.score > b.score;
  });

  if (static_cast<int>(results.size()) > maxResults) {
    results.resize(maxResults);
  }

  return results;
}

}  // namespace diffy
