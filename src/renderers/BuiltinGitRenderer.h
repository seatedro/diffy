#pragma once

#include "renderers/IDiffRenderer.h"

namespace diffy {

class BuiltinGitRenderer : public IDiffRenderer {
 public:
  std::string_view id() const override;
  bool render(const RenderRequest& request, DiffDocument* out, std::string* error) override;
};

}  // namespace diffy
