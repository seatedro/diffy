#include "core/GitHubApi.h"

#include <cstdlib>
#include <cstring>
#include <sstream>
#include <string_view>

#include <curl/curl.h>

namespace diffy {
namespace {

size_t writeCallback(char* ptr, size_t size, size_t nmemb, void* userdata) {
  auto* buffer = static_cast<std::string*>(userdata);
  buffer->append(ptr, size * nmemb);
  return size * nmemb;
}

std::string_view extractJsonString(std::string_view json, std::string_view key) {
  const std::string needle = "\"" + std::string(key) + "\"";
  const size_t keyPos = json.find(needle);
  if (keyPos == std::string_view::npos) return {};

  size_t colonPos = json.find(':', keyPos + needle.size());
  if (colonPos == std::string_view::npos) return {};

  size_t start = json.find('"', colonPos + 1);
  if (start == std::string_view::npos) return {};
  ++start;

  std::string result;
  for (size_t i = start; i < json.size(); ++i) {
    if (json[i] == '\\' && i + 1 < json.size()) {
      result += json[++i];
    } else if (json[i] == '"') {
      return std::string_view(json.data() + start, i - start);
    }
  }
  return {};
}

int extractJsonInt(std::string_view json, std::string_view key) {
  const std::string needle = "\"" + std::string(key) + "\"";
  const size_t keyPos = json.find(needle);
  if (keyPos == std::string_view::npos) return 0;

  size_t colonPos = json.find(':', keyPos + needle.size());
  if (colonPos == std::string_view::npos) return 0;

  size_t start = colonPos + 1;
  while (start < json.size() && (json[start] == ' ' || json[start] == '\t')) ++start;

  int value = 0;
  bool negative = false;
  if (start < json.size() && json[start] == '-') {
    negative = true;
    ++start;
  }
  for (size_t i = start; i < json.size() && json[i] >= '0' && json[i] <= '9'; ++i) {
    value = value * 10 + (json[i] - '0');
  }
  return negative ? -value : value;
}

std::string_view findNestedObject(std::string_view json, std::string_view key) {
  const std::string needle = "\"" + std::string(key) + "\"";
  const size_t keyPos = json.find(needle);
  if (keyPos == std::string_view::npos) return {};

  size_t bracePos = json.find('{', keyPos + needle.size());
  if (bracePos == std::string_view::npos) return {};

  int depth = 1;
  for (size_t i = bracePos + 1; i < json.size(); ++i) {
    if (json[i] == '{') ++depth;
    else if (json[i] == '}') {
      --depth;
      if (depth == 0) {
        return json.substr(bracePos, i - bracePos + 1);
      }
    }
  }
  return {};
}

}  // namespace

std::optional<PullRequestInfo> fetchPullRequest(const std::string& owner,
                                                 const std::string& repo,
                                                 int number,
                                                 const std::string& token,
                                                 std::string* error) {
  const std::string url = "https://api.github.com/repos/" + owner + "/" + repo + "/pulls/" + std::to_string(number);

  CURL* curl = curl_easy_init();
  if (!curl) {
    if (error) *error = "Failed to initialize HTTP client";
    return std::nullopt;
  }

  std::string responseBody;
  struct curl_slist* headers = nullptr;
  headers = curl_slist_append(headers, "Accept: application/vnd.github.v3+json");
  headers = curl_slist_append(headers, "User-Agent: diffy/1.0");

  std::string authHeader;
  if (!token.empty()) {
    authHeader = "Authorization: Bearer " + token;
    headers = curl_slist_append(headers, authHeader.c_str());
  }

  curl_easy_setopt(curl, CURLOPT_URL, url.c_str());
  curl_easy_setopt(curl, CURLOPT_HTTPHEADER, headers);
  curl_easy_setopt(curl, CURLOPT_WRITEFUNCTION, writeCallback);
  curl_easy_setopt(curl, CURLOPT_WRITEDATA, &responseBody);
  curl_easy_setopt(curl, CURLOPT_TIMEOUT, 15L);
  curl_easy_setopt(curl, CURLOPT_FOLLOWLOCATION, 1L);

  const CURLcode res = curl_easy_perform(curl);
  long httpCode = 0;
  curl_easy_getinfo(curl, CURLINFO_RESPONSE_CODE, &httpCode);
  curl_slist_free_all(headers);
  curl_easy_cleanup(curl);

  if (res != CURLE_OK) {
    if (error) *error = std::string("HTTP request failed: ") + curl_easy_strerror(res);
    return std::nullopt;
  }

  if (httpCode == 404) {
    if (error) {
      *error = "Pull request not found: " + owner + "/" + repo + "#" + std::to_string(number);
      if (token.empty()) {
        *error += ". For private repos, enter a GitHub token.";
      }
    }
    return std::nullopt;
  }

  if (httpCode == 403 || httpCode == 401) {
    if (error) *error = "GitHub API authentication required. Set GITHUB_TOKEN environment variable.";
    return std::nullopt;
  }

  if (httpCode != 200) {
    if (error) *error = "GitHub API returned HTTP " + std::to_string(httpCode);
    return std::nullopt;
  }

  const std::string_view json = responseBody;

  PullRequestInfo info;
  info.number = number;
  info.title = std::string(extractJsonString(json, "title"));
  info.state = std::string(extractJsonString(json, "state"));
  info.additions = extractJsonInt(json, "additions");
  info.deletions = extractJsonInt(json, "deletions");
  info.changedFiles = extractJsonInt(json, "changed_files");

  const auto baseObj = findNestedObject(json, "base");
  if (!baseObj.empty()) {
    info.baseBranch = std::string(extractJsonString(baseObj, "ref"));
    info.baseSha = std::string(extractJsonString(baseObj, "sha"));
  }

  const auto headObj = findNestedObject(json, "head");
  if (!headObj.empty()) {
    info.headBranch = std::string(extractJsonString(headObj, "ref"));
    info.headSha = std::string(extractJsonString(headObj, "sha"));
  }

  const auto userObj = findNestedObject(json, "user");
  if (!userObj.empty()) {
    info.authorLogin = std::string(extractJsonString(userObj, "login"));
  }

  if (info.baseBranch.empty() || info.headBranch.empty()) {
    if (error) *error = "Failed to parse PR metadata from GitHub API response";
    return std::nullopt;
  }

  return info;
}

}  // namespace diffy
