#include "core/GitHubDeviceFlow.h"

#include <cstring>
#include <string_view>

#include <curl/curl.h>

#include "core/Log.h"

namespace diffy {
namespace {

size_t writeCallback(char* ptr, size_t size, size_t nmemb, void* userdata) {
  auto* buffer = static_cast<std::string*>(userdata);
  buffer->append(ptr, size * nmemb);
  return size * nmemb;
}

std::string_view extractFormValue(std::string_view body, std::string_view key) {
  const std::string needle = std::string(key) + "=";
  size_t pos = 0;
  while (pos < body.size()) {
    size_t found = body.find(needle, pos);
    if (found == std::string_view::npos) return {};
    if (found == 0 || body[found - 1] == '&') {
      size_t valueStart = found + needle.size();
      size_t valueEnd = body.find('&', valueStart);
      if (valueEnd == std::string_view::npos) valueEnd = body.size();
      return body.substr(valueStart, valueEnd - valueStart);
    }
    pos = found + 1;
  }
  return {};
}

int parseFormInt(std::string_view body, std::string_view key) {
  const auto value = extractFormValue(body, key);
  if (value.empty()) return 0;
  int result = 0;
  for (char c : value) {
    if (c < '0' || c > '9') break;
    result = result * 10 + (c - '0');
  }
  return result;
}

std::string urlDecode(std::string_view input) {
  std::string result;
  result.reserve(input.size());
  for (size_t i = 0; i < input.size(); ++i) {
    if (input[i] == '%' && i + 2 < input.size()) {
      auto hexVal = [](char c) -> int {
        if (c >= '0' && c <= '9') return c - '0';
        if (c >= 'a' && c <= 'f') return 10 + c - 'a';
        if (c >= 'A' && c <= 'F') return 10 + c - 'A';
        return -1;
      };
      int hi = hexVal(input[i + 1]);
      int lo = hexVal(input[i + 2]);
      if (hi >= 0 && lo >= 0) {
        result += static_cast<char>(hi * 16 + lo);
        i += 2;
        continue;
      }
    }
    if (input[i] == '+') {
      result += ' ';
    } else {
      result += input[i];
    }
  }
  return result;
}

}  // namespace

std::optional<DeviceCodeResponse> requestDeviceCode(const std::string& clientId,
                                                     const std::string& scope,
                                                     std::string* error) {
  log::info("github-auth", "requesting device code for client_id={}", clientId);

  CURL* curl = curl_easy_init();
  if (!curl) {
    if (error) *error = "Failed to initialize HTTP client";
    return std::nullopt;
  }

  const std::string postFields = "client_id=" + clientId + "&scope=" + scope;
  std::string responseBody;

  struct curl_slist* headers = nullptr;
  headers = curl_slist_append(headers, "Accept: application/x-www-form-urlencoded");

  curl_easy_setopt(curl, CURLOPT_URL, "https://github.com/login/device/code");
  curl_easy_setopt(curl, CURLOPT_POSTFIELDS, postFields.c_str());
  curl_easy_setopt(curl, CURLOPT_HTTPHEADER, headers);
  curl_easy_setopt(curl, CURLOPT_WRITEFUNCTION, writeCallback);
  curl_easy_setopt(curl, CURLOPT_WRITEDATA, &responseBody);
  curl_easy_setopt(curl, CURLOPT_TIMEOUT, 15L);

  const CURLcode res = curl_easy_perform(curl);
  long httpCode = 0;
  curl_easy_getinfo(curl, CURLINFO_RESPONSE_CODE, &httpCode);
  curl_slist_free_all(headers);
  curl_easy_cleanup(curl);

  if (res != CURLE_OK) {
    const std::string msg = std::string("HTTP request failed: ") + curl_easy_strerror(res);
    log::error("github-auth", "{}", msg);
    if (error) *error = msg;
    return std::nullopt;
  }

  log::info("github-auth", "device code response: HTTP {} ({} bytes)", httpCode, responseBody.size());
  log::debug("github-auth", "body: {}", responseBody);

  if (httpCode != 200) {
    const auto errDesc = extractFormValue(responseBody, "error_description");
    const std::string msg = errDesc.empty()
        ? "GitHub returned HTTP " + std::to_string(httpCode)
        : urlDecode(errDesc);
    log::error("github-auth", "{}", msg);
    if (error) *error = msg;
    return std::nullopt;
  }

  DeviceCodeResponse response;
  response.deviceCode = std::string(extractFormValue(responseBody, "device_code"));
  response.userCode = std::string(extractFormValue(responseBody, "user_code"));
  response.verificationUri = urlDecode(extractFormValue(responseBody, "verification_uri"));
  response.expiresIn = parseFormInt(responseBody, "expires_in");
  response.interval = parseFormInt(responseBody, "interval");
  if (response.interval < 5) response.interval = 5;

  if (response.deviceCode.empty() || response.userCode.empty()) {
    log::error("github-auth", "missing device_code or user_code in response");
    if (error) *error = "Invalid response from GitHub device flow";
    return std::nullopt;
  }

  log::info("github-auth", "user_code={} uri={} interval={}s expires={}s",
            response.userCode, response.verificationUri, response.interval, response.expiresIn);
  return response;
}

PollResponse pollForToken(const std::string& clientId, const std::string& deviceCode) {
  CURL* curl = curl_easy_init();
  if (!curl) {
    return {PollResult::Error, {}, {}, {}, "Failed to initialize HTTP client"};
  }

  const std::string postFields =
      "client_id=" + clientId + "&device_code=" + deviceCode +
      "&grant_type=urn:ietf:params:oauth:grant-type:device_code";
  std::string responseBody;

  struct curl_slist* headers = nullptr;
  headers = curl_slist_append(headers, "Accept: application/x-www-form-urlencoded");

  curl_easy_setopt(curl, CURLOPT_URL, "https://github.com/login/oauth/access_token");
  curl_easy_setopt(curl, CURLOPT_POSTFIELDS, postFields.c_str());
  curl_easy_setopt(curl, CURLOPT_HTTPHEADER, headers);
  curl_easy_setopt(curl, CURLOPT_WRITEFUNCTION, writeCallback);
  curl_easy_setopt(curl, CURLOPT_WRITEDATA, &responseBody);
  curl_easy_setopt(curl, CURLOPT_TIMEOUT, 15L);

  const CURLcode res = curl_easy_perform(curl);
  long httpCode = 0;
  curl_easy_getinfo(curl, CURLINFO_RESPONSE_CODE, &httpCode);
  curl_slist_free_all(headers);
  curl_easy_cleanup(curl);

  if (res != CURLE_OK) {
    return {PollResult::Error, {}, {}, {},
            std::string("HTTP request failed: ") + curl_easy_strerror(res)};
  }

  const auto errorCode = extractFormValue(responseBody, "error");
  if (errorCode == "authorization_pending") {
    return {PollResult::Pending};
  }
  if (errorCode == "slow_down") {
    return {PollResult::SlowDown};
  }
  if (errorCode == "expired_token") {
    return {PollResult::ExpiredToken, {}, {}, {}, "Device code expired. Please try again."};
  }
  if (!errorCode.empty()) {
    const auto desc = urlDecode(extractFormValue(responseBody, "error_description"));
    log::error("github-auth", "poll error: {} — {}", errorCode, desc);
    return {PollResult::Error, {}, {}, {}, desc.empty() ? std::string(errorCode) : desc};
  }

  const auto token = extractFormValue(responseBody, "access_token");
  if (token.empty()) {
    return {PollResult::Error, {}, {}, {}, "No access token in response"};
  }

  log::info("github-auth", "token acquired (scope={})", extractFormValue(responseBody, "scope"));
  return {
      PollResult::Complete,
      std::string(token),
      std::string(extractFormValue(responseBody, "token_type")),
      std::string(extractFormValue(responseBody, "scope")),
  };
}

}  // namespace diffy
