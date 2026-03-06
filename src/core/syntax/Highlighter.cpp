#include "core/syntax/Highlighter.h"

#include <algorithm>
#include <string>
#include <unordered_map>

#include <tree_sitter/api.h>

#include "core/SyntaxTypes.h"

namespace diffy {
namespace {

SyntaxTokenKind captureNameToKind(std::string_view name) {
  if (name.starts_with("keyword")) return SyntaxTokenKind::Keyword;
  if (name.starts_with("string")) return SyntaxTokenKind::String;
  if (name.starts_with("comment")) return SyntaxTokenKind::Comment;
  if (name.starts_with("type")) return SyntaxTokenKind::Type;
  if (name.starts_with("constructor")) return SyntaxTokenKind::Type;
  if (name.starts_with("function")) return SyntaxTokenKind::Function;
  if (name.starts_with("variable")) return SyntaxTokenKind::Variable;
  if (name.starts_with("number")) return SyntaxTokenKind::Number;
  if (name.starts_with("operator")) return SyntaxTokenKind::Operator;
  if (name.starts_with("punctuation")) return SyntaxTokenKind::Punctuation;
  if (name.starts_with("property")) return SyntaxTokenKind::Property;
  if (name.starts_with("attribute")) return SyntaxTokenKind::Attribute;
  if (name.starts_with("module")) return SyntaxTokenKind::Namespace;
  if (name.starts_with("namespace")) return SyntaxTokenKind::Namespace;
  if (name.starts_with("constant")) return SyntaxTokenKind::Constant;
  if (name.starts_with("boolean")) return SyntaxTokenKind::Constant;
  if (name.starts_with("label")) return SyntaxTokenKind::Label;
  if (name.starts_with("embedded")) return SyntaxTokenKind::Embedded;
  if (name.starts_with("escape")) return SyntaxTokenKind::String;
  if (name.starts_with("tag")) return SyntaxTokenKind::Keyword;
  return SyntaxTokenKind::None;
}

}  // namespace

struct Highlighter::Impl {
  TSParser* parser = nullptr;

  Impl() { parser = ts_parser_new(); }

  ~Impl() {
    if (parser != nullptr) {
      ts_parser_delete(parser);
    }
  }
};

Highlighter::Highlighter() : impl_(new Impl) {}

Highlighter::~Highlighter() {
  delete impl_;
}

std::vector<TokenSpan> Highlighter::highlight(const GrammarInfo& grammar, std::string_view source) const {
  if (grammar.language == nullptr || grammar.highlightsQuery.empty() || source.empty()) {
    return {};
  }

  ts_parser_set_language(impl_->parser, grammar.language);

  TSTree* tree = ts_parser_parse_string(impl_->parser, nullptr, source.data(),
                                         static_cast<uint32_t>(source.size()));
  if (tree == nullptr) {
    return {};
  }

  uint32_t errorOffset = 0;
  TSQueryError errorType = TSQueryErrorNone;
  TSQuery* query = ts_query_new(grammar.language, grammar.highlightsQuery.c_str(),
                                 static_cast<uint32_t>(grammar.highlightsQuery.size()),
                                 &errorOffset, &errorType);
  if (query == nullptr) {
    ts_tree_delete(tree);
    return {};
  }

  const uint32_t captureCount = ts_query_capture_count(query);
  std::vector<SyntaxTokenKind> captureKinds(captureCount, SyntaxTokenKind::None);
  for (uint32_t i = 0; i < captureCount; ++i) {
    uint32_t nameLen = 0;
    const char* name = ts_query_capture_name_for_id(query, i, &nameLen);
    captureKinds[i] = captureNameToKind(std::string_view(name, nameLen));
  }

  TSQueryCursor* cursor = ts_query_cursor_new();
  TSNode rootNode = ts_tree_root_node(tree);
  ts_query_cursor_exec(cursor, query, rootNode);

  struct RawSpan {
    uint32_t startByte;
    uint32_t endByte;
    SyntaxTokenKind kind;
    uint16_t patternIndex;
  };
  std::vector<RawSpan> rawSpans;

  TSQueryMatch match;
  uint32_t captureIndex = 0;
  while (ts_query_cursor_next_capture(cursor, &match, &captureIndex)) {
    const TSQueryCapture& capture = match.captures[captureIndex];
    SyntaxTokenKind kind = captureKinds[capture.index];
    if (kind == SyntaxTokenKind::None) {
      continue;
    }

    uint32_t startByte = ts_node_start_byte(capture.node);
    uint32_t endByte = ts_node_end_byte(capture.node);
    if (endByte > startByte) {
      rawSpans.push_back({startByte, endByte, kind, match.pattern_index});
    }
  }

  ts_query_cursor_delete(cursor);
  ts_query_delete(query);
  ts_tree_delete(tree);

  std::sort(rawSpans.begin(), rawSpans.end(), [](const RawSpan& a, const RawSpan& b) {
    if (a.startByte != b.startByte) return a.startByte < b.startByte;
    return a.patternIndex > b.patternIndex;
  });

  std::vector<TokenSpan> result;
  uint32_t covered = 0;
  for (const auto& span : rawSpans) {
    if (span.startByte < covered) {
      continue;
    }
    result.push_back(TokenSpan{
        static_cast<int>(span.startByte),
        static_cast<int>(span.endByte - span.startByte),
        span.kind,
    });
    covered = span.endByte;
  }

  return result;
}

}  // namespace diffy
