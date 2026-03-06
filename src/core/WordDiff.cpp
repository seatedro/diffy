#include "core/WordDiff.h"

#include <algorithm>

namespace diffy {
namespace {

struct Word {
  int start = 0;
  int length = 0;
  std::string_view text;
};

std::vector<Word> tokenize(std::string_view text) {
  std::vector<Word> words;
  int i = 0;
  const int len = static_cast<int>(text.size());
  while (i < len) {
    const int start = i;
    const char ch = text[i];
    if (std::isalnum(static_cast<unsigned char>(ch)) || ch == '_') {
      while (i < len && (std::isalnum(static_cast<unsigned char>(text[i])) || text[i] == '_')) {
        ++i;
      }
    } else if (std::isspace(static_cast<unsigned char>(ch))) {
      while (i < len && std::isspace(static_cast<unsigned char>(text[i]))) {
        ++i;
      }
    } else {
      ++i;
    }
    words.push_back(Word{start, i - start, text.substr(start, i - start)});
  }
  return words;
}

}  // namespace

WordDiffResult computeWordDiff(std::string_view leftText, std::string_view rightText) {
  const auto leftWords = tokenize(leftText);
  const auto rightWords = tokenize(rightText);
  const int m = static_cast<int>(leftWords.size());
  const int n = static_cast<int>(rightWords.size());

  std::vector<std::vector<int>> lcs(m + 1, std::vector<int>(n + 1, 0));
  for (int i = m - 1; i >= 0; --i) {
    for (int j = n - 1; j >= 0; --j) {
      if (leftWords[i].text == rightWords[j].text) {
        lcs[i][j] = lcs[i + 1][j + 1] + 1;
      } else {
        lcs[i][j] = std::max(lcs[i + 1][j], lcs[i][j + 1]);
      }
    }
  }

  WordDiffResult result;
  int i = 0;
  int j = 0;
  while (i < m && j < n) {
    if (leftWords[i].text == rightWords[j].text) {
      ++i;
      ++j;
    } else if (lcs[i + 1][j] >= lcs[i][j + 1]) {
      result.leftTokens.push_back(TokenSpan{leftWords[i].start, leftWords[i].length});
      ++i;
    } else {
      result.rightTokens.push_back(TokenSpan{rightWords[j].start, rightWords[j].length});
      ++j;
    }
  }
  while (i < m) {
    result.leftTokens.push_back(TokenSpan{leftWords[i].start, leftWords[i].length});
    ++i;
  }
  while (j < n) {
    result.rightTokens.push_back(TokenSpan{rightWords[j].start, rightWords[j].length});
    ++j;
  }

  return result;
}

}  // namespace diffy
