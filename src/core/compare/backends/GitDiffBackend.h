#pragma once

#include "core/compare/backends/IDiffBackend.h"

namespace diffy {

class GitDiffBackend : public IDiffBackend {
 public:
  std::string_view id() const override;
  bool compare(const CompareRequest& request, DiffDocument* out, std::string* error) const override;
};

}  // namespace diffy
