#include "core/vcs/github/GitHubPullRequest.h"

#include <charconv>
#include <cctype>

namespace diffy {
namespace {

bool isAsciiSpace(char ch) {
  return std::isspace(static_cast<unsigned char>(ch)) != 0;
}

char toLowerAscii(char ch) {
  return static_cast<char>(std::tolower(static_cast<unsigned char>(ch)));
}

std::string_view trimAscii(std::string_view value) {
  while (!value.empty() && isAsciiSpace(value.front())) {
    value.remove_prefix(1);
  }
  while (!value.empty() && isAsciiSpace(value.back())) {
    value.remove_suffix(1);
  }
  return value;
}

bool startsWithIgnoreCase(std::string_view value, std::string_view prefix) {
  if (value.size() < prefix.size()) {
    return false;
  }
  for (size_t index = 0; index < prefix.size(); ++index) {
    if (toLowerAscii(value[index]) != toLowerAscii(prefix[index])) {
      return false;
    }
  }
  return true;
}

std::string_view nextPathSegment(std::string_view* value) {
  while (!value->empty() && value->front() == '/') {
    value->remove_prefix(1);
  }
  const size_t slash = value->find('/');
  const std::string_view segment = value->substr(0, slash);
  if (slash == std::string_view::npos) {
    value->remove_prefix(value->size());
  } else {
    value->remove_prefix(slash + 1);
  }
  return segment;
}

}  // namespace

std::optional<GitHubPullRequest> parseGitHubPullRequestUrl(std::string_view value) {
  value = trimAscii(value);
  if (value.empty()) {
    return std::nullopt;
  }

  if (startsWithIgnoreCase(value, "https://")) {
    value.remove_prefix(8);
  } else if (startsWithIgnoreCase(value, "http://")) {
    value.remove_prefix(7);
  }

  if (startsWithIgnoreCase(value, "www.")) {
    value.remove_prefix(4);
  }

  if (!startsWithIgnoreCase(value, "github.com/")) {
    return std::nullopt;
  }
  value.remove_prefix(11);

  const size_t queryStart = value.find_first_of("?#");
  if (queryStart != std::string_view::npos) {
    value = value.substr(0, queryStart);
  }
  while (!value.empty() && value.back() == '/') {
    value.remove_suffix(1);
  }

  const std::string_view owner = nextPathSegment(&value);
  const std::string_view repo = nextPathSegment(&value);
  const std::string_view pullLiteral = nextPathSegment(&value);
  const std::string_view numberText = nextPathSegment(&value);

  if (owner.empty() || repo.empty() || pullLiteral != "pull" || numberText.empty()) {
    return std::nullopt;
  }

  int number = 0;
  const auto result = std::from_chars(numberText.data(), numberText.data() + numberText.size(), number);
  if (result.ec != std::errc() || result.ptr != numberText.data() + numberText.size() || number <= 0) {
    return std::nullopt;
  }

  return GitHubPullRequest{
      .owner = std::string(owner),
      .repo = std::string(repo),
      .number = number,
  };
}

}  // namespace diffy
