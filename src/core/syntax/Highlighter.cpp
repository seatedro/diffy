#include "core/syntax/Highlighter.h"

#include <algorithm>
#include <string>
#include <unordered_map>

#include <tree_sitter/api.h>

#include "core/syntax/SyntaxTypes.h"

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

struct CachedQuery {
  TSQuery* query = nullptr;
  std::vector<SyntaxTokenKind> captureKinds;

  CachedQuery() = default;
  CachedQuery(CachedQuery&& other) noexcept
      : query(other.query), captureKinds(std::move(other.captureKinds)) {
    other.query = nullptr;
  }
  CachedQuery& operator=(CachedQuery&& other) noexcept {
    if (this != &other) {
      if (query != nullptr) {
        ts_query_delete(query);
      }
      query = other.query;
      captureKinds = std::move(other.captureKinds);
      other.query = nullptr;
    }
    return *this;
  }
  CachedQuery(const CachedQuery&) = delete;
  CachedQuery& operator=(const CachedQuery&) = delete;

  ~CachedQuery() {
    if (query != nullptr) {
      ts_query_delete(query);
    }
  }
};

struct Highlighter::Impl {
  TSParser* parser = nullptr;
  TSQueryCursor* cursor = nullptr;
  std::unordered_map<const TSLanguage*, CachedQuery> queryCache;

  Impl() {
    parser = ts_parser_new();
    cursor = ts_query_cursor_new();
  }

  ~Impl() {
    if (cursor != nullptr) {
      ts_query_cursor_delete(cursor);
    }
    queryCache.clear();
    if (parser != nullptr) {
      ts_parser_delete(parser);
    }
  }

  CachedQuery* getOrCompileQuery(const GrammarInfo& grammar) {
    auto it = queryCache.find(grammar.language);
    if (it != queryCache.end()) {
      return &it->second;
    }

    uint32_t errorOffset = 0;
    TSQueryError errorType = TSQueryErrorNone;
    TSQuery* query = ts_query_new(grammar.language, grammar.highlightsQuery.data(),
                                   static_cast<uint32_t>(grammar.highlightsQuery.size()),
                                   &errorOffset, &errorType);
    if (query == nullptr) {
      return nullptr;
    }

    CachedQuery cached;
    cached.query = query;
    const uint32_t captureCount = ts_query_capture_count(query);
    cached.captureKinds.resize(captureCount, SyntaxTokenKind::None);
    for (uint32_t i = 0; i < captureCount; ++i) {
      uint32_t nameLen = 0;
      const char* name = ts_query_capture_name_for_id(query, i, &nameLen);
      cached.captureKinds[i] = captureNameToKind(std::string_view(name, nameLen));
    }

    auto [insertIt, _] = queryCache.emplace(grammar.language, std::move(cached));
    return &insertIt->second;
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

  CachedQuery* cached = impl_->getOrCompileQuery(grammar);
  if (cached == nullptr) {
    return {};
  }

  ts_parser_set_language(impl_->parser, grammar.language);

  TSTree* tree = ts_parser_parse_string(impl_->parser, nullptr, source.data(),
                                         static_cast<uint32_t>(source.size()));
  if (tree == nullptr) {
    return {};
  }

  TSNode rootNode = ts_tree_root_node(tree);
  ts_query_cursor_exec(impl_->cursor, cached->query, rootNode);

  struct RawSpan {
    uint32_t startByte;
    uint32_t endByte;
    SyntaxTokenKind kind;
    uint16_t patternIndex;
  };
  std::vector<RawSpan> rawSpans;
  rawSpans.reserve(source.size() / 4);

  TSQueryMatch match;
  uint32_t captureIndex = 0;
  while (ts_query_cursor_next_capture(impl_->cursor, &match, &captureIndex)) {
    const TSQueryCapture& capture = match.captures[captureIndex];
    SyntaxTokenKind kind = cached->captureKinds[capture.index];
    if (kind == SyntaxTokenKind::None) {
      continue;
    }

    uint32_t startByte = ts_node_start_byte(capture.node);
    uint32_t endByte = ts_node_end_byte(capture.node);
    if (endByte > startByte) {
      rawSpans.push_back({startByte, endByte, kind, match.pattern_index});
    }
  }

  ts_tree_delete(tree);

  std::sort(rawSpans.begin(), rawSpans.end(), [](const RawSpan& a, const RawSpan& b) {
    if (a.startByte != b.startByte) return a.startByte < b.startByte;
    return a.patternIndex > b.patternIndex;
  });

  std::vector<TokenSpan> result;
  result.reserve(rawSpans.size());
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
