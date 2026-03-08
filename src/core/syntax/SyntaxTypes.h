#pragma once

#include <cstdint>

namespace diffy {

enum class SyntaxTokenKind : uint8_t {
  None,
  Keyword,
  String,
  Comment,
  Type,
  Function,
  Variable,
  Number,
  Operator,
  Punctuation,
  Property,
  Attribute,
  Namespace,
  Constant,
  Label,
  Embedded,
};

}  // namespace diffy
