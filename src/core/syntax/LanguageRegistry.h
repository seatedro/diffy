#pragma once

#include <string>
#include <string_view>
#include <unordered_map>
#include <vector>

struct TSLanguage;

namespace diffy {

struct GrammarInfo {
  std::string name;
  std::string parserPath;
  std::string highlightsQuery;
  const TSLanguage* language = nullptr;
  void* dlHandle = nullptr;
};

class LanguageRegistry {
 public:
  LanguageRegistry();
  ~LanguageRegistry();

  LanguageRegistry(const LanguageRegistry&) = delete;
  LanguageRegistry& operator=(const LanguageRegistry&) = delete;

  void discoverGrammars(const std::string& searchPaths);
  const GrammarInfo* grammarForExtension(std::string_view extension) const;
  const GrammarInfo* grammarForName(std::string_view name) const;

 private:
  bool loadGrammar(GrammarInfo& info);

  std::vector<GrammarInfo> grammars_;
  std::unordered_map<std::string, size_t> extensionMap_;
  std::unordered_map<std::string, size_t> nameMap_;
};

}  // namespace diffy
