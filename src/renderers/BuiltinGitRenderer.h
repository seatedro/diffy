#pragma once

#include "core/UnifiedDiffParser.h"
#include "renderers/IDiffRenderer.h"

namespace diffy {

class BuiltinGitRenderer : public IDiffRenderer {
 public:
  explicit BuiltinGitRenderer(const UnifiedDiffParser* parser);

  QString id() const override;
  bool render(const RenderRequest& request, DiffDocument* out, QString* error) override;

 private:
  const UnifiedDiffParser* parser_ = nullptr;
};

}  // namespace diffy
