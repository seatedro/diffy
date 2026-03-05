#pragma once

#include <QString>

#include "core/DiffTypes.h"

namespace diffy {

struct RenderRequest {
  QString repoPath;
  QString leftRevision;
  QString rightRevision;
};

class IDiffRenderer {
 public:
  virtual ~IDiffRenderer() = default;

  virtual QString id() const = 0;
  virtual bool render(const RenderRequest& request, DiffDocument* out, QString* error) = 0;
};

}  // namespace diffy
