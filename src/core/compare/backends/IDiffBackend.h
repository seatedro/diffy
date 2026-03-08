#pragma once

#include <string>
#include <string_view>

#include "core/diff/DiffTypes.h"

namespace diffy {

struct CompareRequest {
  std::string repoPath;
  std::string leftRevision;
  std::string rightRevision;
};

class IDiffBackend {
 public:
  virtual ~IDiffBackend() = default;

  virtual std::string_view id() const = 0;
  virtual bool compare(const CompareRequest& request, DiffDocument* out, std::string* error) const = 0;
};

}  // namespace diffy
