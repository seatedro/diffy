#include "core/compare/CompareService.h"

namespace diffy {

CompareOutput CompareService::compare(const CompareRequest& request, std::string_view backendId) const {
  CompareOutput result;

  const IDiffBackend* backend = &gitBackend_;
  const bool useDifftastic = backendId == difftasticBackend_.id();
  if (useDifftastic) {
    backend = &difftasticBackend_;
  }

  DiffDocument document;
  std::string compareError;
  const bool compared = backend->compare(request, &document, &compareError);
  if (!compared && useDifftastic) {
    DiffDocument fallback;
    std::string fallbackError;
    if (gitBackend_.compare(request, &fallback, &fallbackError)) {
      document = std::move(fallback);
      result.usedFallback = true;
      result.fallbackMessage = "difftastic failed (" + compareError + "). Fell back to built-in backend.";
    } else {
      result.errorMessage =
          "difftastic failed (" + compareError + "); built-in fallback failed (" + fallbackError + ")";
      return result;
    }
  } else if (!compared) {
    result.errorMessage = compareError;
    return result;
  }

  result.fileDiffs = std::move(document.files);
  return result;
}

}  // namespace diffy
