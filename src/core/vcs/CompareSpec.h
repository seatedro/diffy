#pragma once

#include <string>
#include <string_view>

namespace diffy {

enum class CompareMode {
  TwoDot,
  ThreeDot,
  SingleCommit,
};

enum class LayoutMode {
  Unified,
  Split,
};

enum class RendererKind {
  Builtin,
  Difftastic,
};

struct CompareSpec {
  std::string repoPath;
  std::string leftRef;
  std::string rightRef;
  CompareMode mode = CompareMode::TwoDot;
  LayoutMode layout = LayoutMode::Unified;
  RendererKind renderer = RendererKind::Builtin;
};

std::string_view compareModeToString(CompareMode mode);
CompareMode compareModeFromString(std::string_view value);

std::string_view layoutModeToString(LayoutMode mode);
LayoutMode layoutModeFromString(std::string_view value);

std::string_view rendererKindToString(RendererKind kind);
RendererKind rendererKindFromString(std::string_view value);

}  // namespace diffy
