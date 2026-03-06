#pragma once

#include <string>
#include <string_view>
#include <unordered_map>
#include <vector>

struct TSLanguage;

namespace diffy {

struct GrammarInfo {
  std::string name;
  std::string_view highlightsQuery;
  const TSLanguage* language = nullptr;
};

class LanguageRegistry {
 public:
  LanguageRegistry();

  void loadBuiltinGrammars();
  const GrammarInfo* grammarForExtension(std::string_view extension) const;
  const GrammarInfo* grammarForName(std::string_view name) const;

 private:
  std::vector<GrammarInfo> grammars_;
  std::unordered_map<std::string, size_t> extensionMap_;
  std::unordered_map<std::string, size_t> nameMap_;
};

}  // namespace diffy
