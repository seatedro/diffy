#pragma once

#include <string_view>

#include "core/DiffTypes.h"

namespace diffy {

class UnifiedDiffParser {
 public:
  DiffDocument parse(std::string_view leftRevision,
                     std::string_view rightRevision,
                     std::string_view diffText) const;
};

}  // namespace diffy
