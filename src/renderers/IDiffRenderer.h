#pragma once

#include <string>
#include <string_view>

#include "core/DiffTypes.h"

namespace diffy {

struct RenderRequest {
  std::string repoPath;
  std::string leftRevision;
  std::string rightRevision;
};

class IDiffRenderer {
 public:
  virtual ~IDiffRenderer() = default;

  virtual std::string_view id() const = 0;
  virtual bool render(const RenderRequest& request, DiffDocument* out, std::string* error) = 0;
};

}  // namespace diffy
