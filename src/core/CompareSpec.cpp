#include "core/CompareSpec.h"

namespace diffy {

QString compareModeToString(CompareMode mode) {
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

CompareMode compareModeFromString(const QString& value) {
  if (value == "three-dot") {
    return CompareMode::ThreeDot;
  }
  if (value == "single-commit") {
    return CompareMode::SingleCommit;
  }
  return CompareMode::TwoDot;
}

QString layoutModeToString(LayoutMode mode) {
  if (mode == LayoutMode::Split) {
    return "split";
  }
  return "unified";
}

LayoutMode layoutModeFromString(const QString& value) {
  if (value == "split") {
    return LayoutMode::Split;
  }
  return LayoutMode::Unified;
}

QString rendererKindToString(RendererKind kind) {
  if (kind == RendererKind::Difftastic) {
    return "difftastic";
  }
  return "builtin";
}

RendererKind rendererKindFromString(const QString& value) {
  if (value == "difftastic") {
    return RendererKind::Difftastic;
  }
  return RendererKind::Builtin;
}

}  // namespace diffy
