#pragma once

#include <optional>
#include <string>
#include <string_view>

namespace diffy {

struct GitHubPullRequest {
  std::string owner;
  std::string repo;
  int number = 0;
};

std::optional<GitHubPullRequest> parseGitHubPullRequestUrl(std::string_view value);

}  // namespace diffy
