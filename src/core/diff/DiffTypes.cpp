#include "core/diff/DiffTypes.h"

namespace diffy {

std::string_view lineKindToString(LineKind kind) {
  switch (kind) {
    case LineKind::Addition:
      return "add";
    case LineKind::Deletion:
      return "del";
    case LineKind::Context:
      return "ctx";
  }
  return "ctx";
}

LineKind lineKindFromString(std::string_view value) {
  if (value == "add") {
    return LineKind::Addition;
  }
  if (value == "del") {
    return LineKind::Deletion;
  }
  return LineKind::Context;
}

}  // namespace diffy
