#pragma once

#include "renderers/IDiffRenderer.h"

namespace diffy {

class BuiltinGitRenderer : public IDiffRenderer {
 public:
  QString id() const override;
  bool render(const RenderRequest& request, DiffDocument* out, QString* error) override;
};

}  // namespace diffy
