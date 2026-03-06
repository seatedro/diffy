#pragma once

#include "renderers/IDiffRenderer.h"

namespace diffy {

class Highlighter;
class LanguageRegistry;

class BuiltinGitRenderer : public IDiffRenderer {
 public:
  std::string_view id() const override;
  bool render(const RenderRequest& request, DiffDocument* out, std::string* error) override;

  void setSyntax(const LanguageRegistry* registry, const Highlighter* highlighter);

 private:
  void applySyntaxHighlighting(DiffDocument& document) const;

  const LanguageRegistry* languageRegistry_ = nullptr;
  const Highlighter* highlighter_ = nullptr;
};

}  // namespace diffy
