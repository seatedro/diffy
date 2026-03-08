#include "core/vcs/CompareSpec.h"

namespace diffy {

std::string_view compareModeToString(CompareMode mode) {
  switch (mode) {
    case CompareMode::TwoDot:
      return "two-dot";
    case CompareMode::ThreeDot:
      return "three-dot";
    case CompareMode::SingleCommit:
      return "single-commit";
  }
  return "two-dot";
}

CompareMode compareModeFromString(std::string_view value) {
  if (value == "three-dot") {
    return CompareMode::ThreeDot;
  }
  if (value == "single-commit") {
    return CompareMode::SingleCommit;
  }
  return CompareMode::TwoDot;
}

std::string_view layoutModeToString(LayoutMode mode) {
  if (mode == LayoutMode::Split) {
    return "split";
  }
  return "unified";
}

LayoutMode layoutModeFromString(std::string_view value) {
  if (value == "split") {
    return LayoutMode::Split;
  }
  return LayoutMode::Unified;
}

std::string_view rendererKindToString(RendererKind kind) {
  if (kind == RendererKind::Difftastic) {
    return "difftastic";
  }
  return "builtin";
}

RendererKind rendererKindFromString(std::string_view value) {
  if (value == "difftastic") {
    return RendererKind::Difftastic;
  }
  return RendererKind::Builtin;
}

}  // namespace diffy
