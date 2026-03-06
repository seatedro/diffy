#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VENDOR_DIR="$ROOT_DIR/vendor/grammars"
OUTPUT="$ROOT_DIR/src/core/syntax/GrammarData.gen.h"

LANGS=(bash c cpp go javascript json nix python rust toml zig)

cat > "$OUTPUT" << 'HEADER'
#pragma once

#include <string_view>

struct TSLanguage;

HEADER

for lang in "${LANGS[@]}"; do
  symbol="tree_sitter_${lang}"
  echo "extern \"C\" const TSLanguage* ${symbol}(void);" >> "$OUTPUT"
done

echo "" >> "$OUTPUT"
echo "namespace diffy {" >> "$OUTPUT"
echo "namespace grammar_data {" >> "$OUTPUT"
echo "" >> "$OUTPUT"

declare -A INHERITS=(
  [cpp]="c"
)

for lang in "${LANGS[@]}"; do
  varname="kHighlights_${lang}"
  query_file="$VENDOR_DIR/$lang/queries/highlights.scm"
  parent="${INHERITS[$lang]:-}"
  parent_file="$VENDOR_DIR/$parent/queries/highlights.scm"

  echo "constexpr std::string_view ${varname} = R\"QUERY(" >> "$OUTPUT"
  if [[ -n "$parent" && -f "$parent_file" ]]; then
    cat "$parent_file" >> "$OUTPUT"
    echo "" >> "$OUTPUT"
  fi
  if [[ -f "$query_file" ]]; then
    cat "$query_file" >> "$OUTPUT"
  fi
  echo ")QUERY\";" >> "$OUTPUT"
  echo "" >> "$OUTPUT"
done

cat >> "$OUTPUT" << 'STRUCT'
struct GrammarEntry {
  const char* name;
  const TSLanguage* (*languageFn)();
  std::string_view highlightsQuery;
};

STRUCT

echo "constexpr GrammarEntry kGrammars[] = {" >> "$OUTPUT"
for lang in "${LANGS[@]}"; do
  symbol="tree_sitter_${lang}"
  varname="kHighlights_${lang}"
  echo "    {\"${lang}\", ${symbol}, ${varname}}," >> "$OUTPUT"
done
echo "};" >> "$OUTPUT"
echo "" >> "$OUTPUT"
echo "}  // namespace grammar_data" >> "$OUTPUT"
echo "}  // namespace diffy" >> "$OUTPUT"
