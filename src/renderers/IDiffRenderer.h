#pragma once

#include <string>

#include <QString>

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

  virtual QString id() const = 0;
  virtual bool render(const RenderRequest& request, DiffDocument* out, QString* error) = 0;
};

}  // namespace diffy
