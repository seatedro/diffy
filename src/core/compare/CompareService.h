#pragma once

#include <string>

#include "core/compare/backends/DifftasticBackend.h"
#include "core/compare/backends/GitDiffBackend.h"

namespace diffy {

struct CompareOutput {
  std::vector<FileDiff> fileDiffs;
  std::string errorMessage;
  bool usedFallback = false;
  std::string fallbackMessage;
};

class CompareService {
 public:
  CompareOutput compare(const CompareRequest& request, std::string_view backendId) const;

 private:
  GitDiffBackend gitBackend_;
  DifftasticBackend difftasticBackend_;
};

}  // namespace diffy
