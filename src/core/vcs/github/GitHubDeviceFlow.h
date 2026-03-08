#pragma once

#include <optional>
#include <string>

namespace diffy {

struct DeviceCodeResponse {
  std::string deviceCode;
  std::string userCode;
  std::string verificationUri;
  int expiresIn = 0;
  int interval = 5;
};

enum class PollResult { Pending, Complete, SlowDown, ExpiredToken, Error };

struct PollResponse {
  PollResult result = PollResult::Error;
  std::string accessToken;
  std::string tokenType;
  std::string scope;
  std::string error;
};

std::optional<DeviceCodeResponse> requestDeviceCode(const std::string& clientId,
                                                     const std::string& scope,
                                                     std::string* error);

PollResponse pollForToken(const std::string& clientId, const std::string& deviceCode);

}  // namespace diffy
