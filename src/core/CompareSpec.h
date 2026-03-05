#pragma once

#include <QString>

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
  QString repoPath;
  QString leftRef;
  QString rightRef;
  CompareMode mode = CompareMode::TwoDot;
  LayoutMode layout = LayoutMode::Unified;
  RendererKind renderer = RendererKind::Builtin;
};

QString compareModeToString(CompareMode mode);
CompareMode compareModeFromString(const QString& value);

QString layoutModeToString(LayoutMode mode);
LayoutMode layoutModeFromString(const QString& value);

QString rendererKindToString(RendererKind kind);
RendererKind rendererKindFromString(const QString& value);

}  // namespace diffy
