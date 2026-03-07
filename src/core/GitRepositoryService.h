#pragma once

#include <string>
#include <string_view>
#include <vector>

#include <git2.h>

#include "core/CompareSpec.h"

namespace diffy {

class GitRepositoryService {
 public:
  GitRepositoryService();
  ~GitRepositoryService();

  GitRepositoryService(const GitRepositoryService&) = delete;
  GitRepositoryService& operator=(const GitRepositoryService&) = delete;

  bool openRepository(const std::string& path, std::string* error);
  bool isOpen() const;
  std::string repositoryPath() const;

  std::vector<std::string> listReferences(std::string* error) const;

  struct BranchInfo {
    std::string name;
    bool isRemote = false;
    bool isHead = false;
  };
  std::vector<BranchInfo> listBranches(std::string* error) const;

  struct CommitInfo {
    std::string oid;
    std::string summary;
    std::string authorName;
    int64_t timestamp = 0;
  };
  std::vector<CommitInfo> listCommits(std::string_view ref, int limit, std::string* error) const;

  bool resolveComparison(std::string_view leftRef,
                         std::string_view rightRef,
                         CompareMode mode,
                         std::string* outLeftRevision,
                         std::string* outRightRevision,
                         std::string* error) const;

  std::string resolveOidToBranchName(const std::string& oidHex) const;

  bool resolvePullRequestComparison(const std::string& pullRequestUrl,
                                    std::string* outLeftRevision,
                                    std::string* outRightRevision,
                                    std::string* error) const;

 private:
  git_repository* repo_ = nullptr;
  std::string repositoryPath_;
};

}  // namespace diffy
