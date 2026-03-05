#pragma once

#include <QString>

#include "core/DiffTypes.h"

namespace diffy {

class UnifiedDiffParser {
 public:
  DiffDocument parse(const QString& leftRevision,
                     const QString& rightRevision,
                     const QString& diffText) const;
};

}  // namespace diffy
