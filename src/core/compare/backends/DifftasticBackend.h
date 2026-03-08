#pragma once

#include <string_view>

#include "core/compare/backends/IDiffBackend.h"

namespace diffy {

class DifftasticBackend : public IDiffBackend {
 public:
  static bool isAvailable();

 std::string_view id() const override;
  bool compare(const CompareRequest& request, DiffDocument* out, std::string* error) const override;

 private:
  bool parseDifftasticJson(std::string_view json,
                           std::string_view fallbackPath,
                           std::string_view fallbackStatus,
                           FileDiff* outFile,
                           std::string* error) const;
};

}  // namespace diffy
