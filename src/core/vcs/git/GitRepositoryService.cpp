#include "core/vcs/git/GitRepositoryService.h"

#include <algorithm>
#include <array>
#include <cctype>
#include <optional>

#include "core/vcs/github/GitHubPullRequest.h"

namespace diffy {
namespace {

std::string lastGitErrorString(const std::string& fallback) {
  if (const git_error* err = git_error_last(); err && err->message) {
    return err->message;
  }
  return fallback;
}

std::string oidToStdString(const git_oid& oid) {
  char out[GIT_OID_HEXSZ + 1] = {0};
  git_oid_fmt(out, &oid);
  out[GIT_OID_HEXSZ] = '\0';
  return out;
}

char toLowerAscii(char ch) {
  return static_cast<char>(std::tolower(static_cast<unsigned char>(ch)));
}

std::string toLowerAscii(std::string value) {
  for (char& ch : value) {
    ch = toLowerAscii(ch);
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

std::string normalizeGitHubRepoName(std::string value) {
  if (value.size() >= 4 && value.substr(value.size() - 4) == ".git") {
    value.resize(value.size() - 4);
  }
  while (!value.empty() && value.back() == '/') {
    value.pop_back();
  }
  return toLowerAscii(std::move(value));
}

std::optional<std::string> parseGitHubRemoteSlug(std::string_view value) {
  while (!value.empty() && std::isspace(static_cast<unsigned char>(value.front())) != 0) {
    value.remove_prefix(1);
  }
  while (!value.empty() && std::isspace(static_cast<unsigned char>(value.back())) != 0) {
    value.remove_suffix(1);
  }

  std::string slug;
  if (startsWithIgnoreCase(value, "git@github.com:")) {
    slug = std::string(value.substr(15));
  } else if (startsWithIgnoreCase(value, "ssh://git@github.com/")) {
    slug = std::string(value.substr(21));
  } else if (startsWithIgnoreCase(value, "https://github.com/")) {
    slug = std::string(value.substr(19));
  } else if (startsWithIgnoreCase(value, "http://github.com/")) {
    slug = std::string(value.substr(18));
  } else {
    return std::nullopt;
  }

  const size_t queryStart = slug.find_first_of("?#");
  if (queryStart != std::string::npos) {
    slug.resize(queryStart);
  }

  slug = normalizeGitHubRepoName(std::move(slug));
  const size_t firstSlash = slug.find('/');
  if (firstSlash == std::string::npos) {
    return std::nullopt;
  }
  const size_t secondSlash = slug.find('/', firstSlash + 1);
  if (secondSlash != std::string::npos) {
    slug.resize(secondSlash);
  }
  return slug;
}

std::string localPullRequestHeadRef(int number) {
  return "refs/diffy/pull/" + std::to_string(number) + "/head";
}

std::string localPullRequestMergeRef(int number) {
  return "refs/diffy/pull/" + std::to_string(number) + "/merge";
}

std::optional<std::string> repositoryConfigValue(git_repository* repo, std::string_view name) {
  git_config* config = nullptr;
  git_buf value = GIT_BUF_INIT;

  if (git_repository_config_snapshot(&config, repo) != 0) {
    return std::nullopt;
  }
  const std::string key(name);
  const int getResult = git_config_get_string_buf(&value, config, key.c_str());
  git_config_free(config);

  if (getResult != 0) {
    git_buf_dispose(&value);
    return std::nullopt;
  }

  std::string result;
  if (value.ptr != nullptr && value.size > 0) {
    result.assign(value.ptr, value.size);
  }
  git_buf_dispose(&value);
  return result;
}

int acquireRemoteCredentials(git_credential** out,
                             const char*,
                             const char* usernameFromUrl,
                             unsigned int allowedTypes,
                             void*) {
  const char* username = (usernameFromUrl != nullptr && usernameFromUrl[0] != '\0') ? usernameFromUrl : "git";

  if ((allowedTypes & GIT_CREDENTIAL_USERNAME) != 0) {
    return git_credential_username_new(out, username);
  }
  if ((allowedTypes & GIT_CREDENTIAL_SSH_KEY) != 0) {
    return git_credential_ssh_key_from_agent(out, username);
  }
  if ((allowedTypes & GIT_CREDENTIAL_DEFAULT) != 0) {
    return git_credential_default_new(out);
  }
  return GIT_PASSTHROUGH;
}

bool fetchRefspec(git_remote* remote, const std::string& refspec, std::string* error) {
  git_fetch_options options = GIT_FETCH_OPTIONS_INIT;
  options.download_tags = GIT_REMOTE_DOWNLOAD_TAGS_NONE;
  options.prune = GIT_FETCH_NO_PRUNE;
  options.callbacks.credentials = acquireRemoteCredentials;

  std::array<char*, 1> refspecData{const_cast<char*>(refspec.c_str())};
  git_strarray refspecs{refspecData.data(), refspecData.size()};
  if (git_remote_fetch(remote, &refspecs, &options, "diffy pull request fetch") != 0) {
    if (error != nullptr) {
      *error = lastGitErrorString("Failed to fetch pull request refs");
    }
    return false;
  }
  return true;
}

bool resolveToCommitOid(git_repository* repo, const char* ref, git_oid* out, std::string* error) {
  git_object* object = nullptr;
  git_object* peeled = nullptr;

  if (git_revparse_single(&object, repo, ref) != 0) {
    if (error != nullptr) {
      *error = lastGitErrorString(std::string("Failed to resolve reference: ") + ref);
    }
    return false;
  }

  if (git_object_peel(&peeled, object, GIT_OBJECT_COMMIT) != 0) {
    git_object_free(object);
    if (error != nullptr) {
      *error = lastGitErrorString(std::string("Reference is not a commit: ") + ref);
    }
    return false;
  }

  const git_oid* oid = git_object_id(peeled);
  git_oid_cpy(out, oid);

  git_object_free(peeled);
  git_object_free(object);
  return true;
}

bool resolveToCommitOid(git_repository* repo, std::string_view ref, git_oid* out, std::string* error) {
  const std::string refString(ref);
  return resolveToCommitOid(repo, refString.c_str(), out, error);
}

}  // namespace

GitRepositoryService::GitRepositoryService() {
  git_libgit2_init();
}

GitRepositoryService::~GitRepositoryService() {
  if (repo_ != nullptr) {
    git_repository_free(repo_);
    repo_ = nullptr;
  }
  git_libgit2_shutdown();
}

bool GitRepositoryService::openRepository(const std::string& path, std::string* error) {
  if (repo_ != nullptr) {
    git_repository_free(repo_);
    repo_ = nullptr;
  }

  if (git_repository_open_ext(&repo_, path.c_str(), 0, nullptr) != 0) {
    if (error) {
      *error = lastGitErrorString("Failed to open repository: " + path);
    }
    return false;
  }

  repositoryPath_ = path;
  return true;
}

bool GitRepositoryService::isOpen() const {
  return repo_ != nullptr;
}

std::string GitRepositoryService::repositoryPath() const {
  return repositoryPath_;
}

std::vector<std::string> GitRepositoryService::listReferences(std::string* error) const {
  std::vector<std::string> refs;
  if (repo_ == nullptr) {
    if (error) {
      *error = "Repository is not open";
    }
    return refs;
  }

  git_reference_iterator* iterator = nullptr;
  if (git_reference_iterator_new(&iterator, repo_) != 0) {
    if (error) {
      *error = lastGitErrorString("Failed to iterate references");
    }
    return refs;
  }

  std::vector<std::string> uniqueRefs;
  git_reference* reference = nullptr;
  while (git_reference_next(&reference, iterator) == 0) {
    const char* shorthand = git_reference_shorthand(reference);
    if (shorthand != nullptr) {
      uniqueRefs.emplace_back(shorthand);
    }
    git_reference_free(reference);
  }

  git_reference_iterator_free(iterator);

  std::sort(uniqueRefs.begin(), uniqueRefs.end());
  uniqueRefs.erase(std::unique(uniqueRefs.begin(), uniqueRefs.end()), uniqueRefs.end());
  refs = std::move(uniqueRefs);
  return refs;
}

std::vector<GitRepositoryService::BranchInfo> GitRepositoryService::listBranches(std::string* error) const {
  std::vector<BranchInfo> branches;
  if (repo_ == nullptr) {
    if (error) *error = "Repository is not open";
    return branches;
  }

  git_branch_iterator* iter = nullptr;
  if (git_branch_iterator_new(&iter, repo_, GIT_BRANCH_ALL) != 0) {
    if (error) *error = lastGitErrorString("Failed to iterate branches");
    return branches;
  }

  git_reference* ref = nullptr;
  git_branch_t type{};
  while (git_branch_next(&ref, &type, iter) == 0) {
    const char* name = nullptr;
    if (git_branch_name(&name, ref) == 0 && name != nullptr) {
      BranchInfo info;
      info.name = name;
      info.isRemote = (type == GIT_BRANCH_REMOTE);
      info.isHead = (git_branch_is_head(ref) == 1);
      branches.push_back(std::move(info));
    }
    git_reference_free(ref);
  }
  git_branch_iterator_free(iter);

  std::sort(branches.begin(), branches.end(), [](const BranchInfo& a, const BranchInfo& b) {
    if (a.isHead != b.isHead) return a.isHead;
    if (a.isRemote != b.isRemote) return !a.isRemote;
    return a.name < b.name;
  });

  return branches;
}

std::vector<GitRepositoryService::CommitInfo> GitRepositoryService::listCommits(
    std::string_view ref, int limit, std::string* error) const {
  std::vector<CommitInfo> commits;
  if (repo_ == nullptr) {
    if (error) *error = "Repository is not open";
    return commits;
  }

  git_oid startOid{};
  if (!resolveToCommitOid(repo_, ref, &startOid, error)) {
    return commits;
  }

  git_revwalk* walk = nullptr;
  if (git_revwalk_new(&walk, repo_) != 0) {
    if (error) *error = lastGitErrorString("Failed to create revwalk");
    return commits;
  }

  git_revwalk_sorting(walk, GIT_SORT_TIME);
  git_revwalk_push(walk, &startOid);

  git_oid oid{};
  int count = 0;
  while (count < limit && git_revwalk_next(&oid, walk) == 0) {
    git_commit* commit = nullptr;
    if (git_commit_lookup(&commit, repo_, &oid) != 0) continue;

    CommitInfo info;
    info.oid = oidToStdString(oid);
    const char* summary = git_commit_summary(commit);
    if (summary) info.summary = summary;
    const git_signature* author = git_commit_author(commit);
    if (author && author->name) info.authorName = author->name;
    info.timestamp = git_commit_time(commit);
    commits.push_back(std::move(info));

    git_commit_free(commit);
    ++count;
  }

  git_revwalk_free(walk);
  return commits;
}

std::string GitRepositoryService::resolveOidToBranchName(const std::string& oidHex) const {
  if (repo_ == nullptr || oidHex.size() != GIT_OID_HEXSZ) {
    return {};
  }

  git_oid targetOid{};
  if (git_oid_fromstr(&targetOid, oidHex.c_str()) != 0) {
    return {};
  }

  git_branch_iterator* iter = nullptr;
  if (git_branch_iterator_new(&iter, repo_, GIT_BRANCH_LOCAL) != 0) {
    return {};
  }

  std::string result;
  git_reference* ref = nullptr;
  git_branch_t type{};
  while (git_branch_next(&ref, &type, iter) == 0) {
    git_reference* resolved = nullptr;
    if (git_reference_resolve(&resolved, ref) == 0) {
      const git_oid* tipOid = git_reference_target(resolved);
      if (tipOid != nullptr && git_oid_equal(tipOid, &targetOid)) {
        const char* name = nullptr;
        if (git_branch_name(&name, ref) == 0 && name != nullptr) {
          result = name;
        }
        git_reference_free(resolved);
        git_reference_free(ref);
        break;
      }
      git_reference_free(resolved);
    }
    git_reference_free(ref);
  }
  git_branch_iterator_free(iter);

  return result;
}

bool GitRepositoryService::resolveComparison(std::string_view leftRef,
                                             std::string_view rightRef,
                                             CompareMode mode,
                                             std::string* outLeftRevision,
                                             std::string* outRightRevision,
                                             std::string* error) const {
  if (repo_ == nullptr) {
    if (error) {
      *error = "Repository is not open";
    }
    return false;
  }

  git_oid leftOid{};
  git_oid rightOid{};

  if (mode == CompareMode::SingleCommit) {
    const std::string_view commitRef = rightRef.empty() ? leftRef : rightRef;
    if (commitRef.empty()) {
      if (error) {
        *error = "Single-commit mode requires a commit reference";
      }
      return false;
    }

    if (!resolveToCommitOid(repo_, commitRef, &rightOid, error)) {
      return false;
    }

    git_commit* commit = nullptr;
    if (git_commit_lookup(&commit, repo_, &rightOid) != 0) {
      if (error) {
        *error = lastGitErrorString("Failed to load commit");
      }
      return false;
    }

    if (git_commit_parentcount(commit) == 0) {
      git_commit_free(commit);
      if (error) {
        *error = "Cannot diff the root commit in single-commit mode yet";
      }
      return false;
    }

    const git_oid* parentOid = git_commit_parent_id(commit, 0);
    git_oid_cpy(&leftOid, parentOid);
    git_commit_free(commit);

    if (outLeftRevision) {
      *outLeftRevision = oidToStdString(leftOid);
    }
    if (outRightRevision) {
      *outRightRevision = oidToStdString(rightOid);
    }
    return true;
  }

  if (leftRef.empty() || rightRef.empty()) {
    if (error) {
      *error = "Comparison requires both left and right references";
    }
    return false;
  }

  if (!resolveToCommitOid(repo_, leftRef, &leftOid, error)) {
    return false;
  }
  if (!resolveToCommitOid(repo_, rightRef, &rightOid, error)) {
    return false;
  }

  if (mode == CompareMode::ThreeDot) {
    git_oid baseOid{};
    if (git_merge_base(&baseOid, repo_, &leftOid, &rightOid) != 0) {
      if (error) {
        *error = lastGitErrorString("Failed to resolve merge base");
      }
      return false;
    }
    leftOid = baseOid;
  }

  if (outLeftRevision) {
    *outLeftRevision = oidToStdString(leftOid);
  }
  if (outRightRevision) {
    *outRightRevision = oidToStdString(rightOid);
  }
  return true;
}

bool GitRepositoryService::resolvePullRequestComparison(const std::string& pullRequestUrl,
                                                        std::string* outLeftRevision,
                                                        std::string* outRightRevision,
                                                        std::string* error) const {
  if (repo_ == nullptr) {
    if (error != nullptr) {
      *error = "Repository is not open";
    }
    return false;
  }

  const std::optional<GitHubPullRequest> pullRequest = parseGitHubPullRequestUrl(pullRequestUrl);
  if (!pullRequest.has_value()) {
    if (error != nullptr) {
      *error = "Not a valid GitHub pull request URL";
    }
    return false;
  }

  const std::string targetSlug = toLowerAscii(pullRequest->owner + "/" + pullRequest->repo);

  std::string matchedRemoteName;
  git_strarray remoteNames{};
  if (git_remote_list(&remoteNames, repo_) != 0) {
    if (error != nullptr) {
      *error = lastGitErrorString("Failed to list git remotes");
    }
    return false;
  }

  for (size_t index = 0; index < remoteNames.count; ++index) {
    const std::string remoteName(remoteNames.strings[index]);
    const auto remoteUrl =
        repositoryConfigValue(repo_, std::string("remote.") + remoteName + ".url");
    if (remoteUrl.has_value()) {
      const auto remoteSlug = parseGitHubRemoteSlug(*remoteUrl);
      if (remoteSlug.has_value() && *remoteSlug == targetSlug) {
        matchedRemoteName = remoteName;
        break;
      }
    }
  }
  git_strarray_dispose(&remoteNames);

  if (matchedRemoteName.empty()) {
    if (error != nullptr) {
      *error = "Open a local clone of " + targetSlug + " before loading PR #" +
               std::to_string(pullRequest->number);
    }
    return false;
  }

  git_remote* remote = nullptr;
  if (git_remote_lookup(&remote, repo_, matchedRemoteName.c_str()) != 0) {
    if (error != nullptr) {
      *error = lastGitErrorString("Failed to open the matching remote");
    }
    return false;
  }

  const std::string headRef = localPullRequestHeadRef(pullRequest->number);
  const std::string mergeRef = localPullRequestMergeRef(pullRequest->number);
  const std::string headRefspec =
      "+refs/pull/" + std::to_string(pullRequest->number) + "/head:" + headRef;
  const std::string mergeRefspec =
      "+refs/pull/" + std::to_string(pullRequest->number) + "/merge:" + mergeRef;

  std::string fetchError;
  const bool headFetched = fetchRefspec(remote, headRefspec, &fetchError);
  if (headFetched) {
    fetchError.clear();
    fetchRefspec(remote, mergeRefspec, &fetchError);
  }
  git_remote_free(remote);

  if (!headFetched || !fetchError.empty()) {
    if (error != nullptr) {
      *error = "Failed to prepare PR #" + std::to_string(pullRequest->number) + " from " + targetSlug +
               ". " + (fetchError.empty() ? "Missing PR refs" : fetchError);
    }
    return false;
  }

  git_oid baseOid{};
  git_oid headOid{};
  const std::string mergeParentRef = mergeRef + "^1";
  if (!resolveToCommitOid(repo_, mergeParentRef.c_str(), &baseOid, error)) {
    return false;
  }
  if (!resolveToCommitOid(repo_, headRef.c_str(), &headOid, error)) {
    return false;
  }

  if (outLeftRevision != nullptr) {
    *outLeftRevision = oidToStdString(baseOid);
  }
  if (outRightRevision != nullptr) {
    *outRightRevision = oidToStdString(headOid);
  }
  return true;
}

}  // namespace diffy
