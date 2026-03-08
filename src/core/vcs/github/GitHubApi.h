#pragma once

#include <optional>
#include <string>

namespace diffy {

struct PullRequestInfo {
  std::string title;
  std::string baseBranch;
  std::string headBranch;
  std::string baseSha;
  std::string headSha;
  std::string state;
  std::string authorLogin;
  int number = 0;
  int additions = 0;
  int deletions = 0;
  int changedFiles = 0;
};

std::optional<PullRequestInfo> fetchPullRequest(const std::string& owner,
                                                 const std::string& repo,
                                                 int number,
                                                 const std::string& token,
                                                 std::string* error);

}  // namespace diffy
