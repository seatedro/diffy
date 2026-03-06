#include "core/syntax/LanguageRegistry.h"

#include <tree_sitter/api.h>

#include "core/syntax/GrammarData.gen.h"

namespace diffy {
namespace {

struct ExtensionEntry {
  const char* extension;
  const char* grammarName;
};

constexpr ExtensionEntry kExtensionTable[] = {
    {".c", "c"},
    {".h", "c"},
    {".cc", "cpp"},
    {".cpp", "cpp"},
    {".cxx", "cpp"},
    {".hh", "cpp"},
    {".hpp", "cpp"},
    {".hxx", "cpp"},
    {".rs", "rust"},
    {".py", "python"},
    {".pyi", "python"},
    {".js", "javascript"},
    {".jsx", "javascript"},
    {".mjs", "javascript"},
    {".go", "go"},
    {".sh", "bash"},
    {".bash", "bash"},
    {".zsh", "bash"},
    {".json", "json"},
    {".toml", "toml"},
    {".zig", "zig"},
    {".nix", "nix"},
};

}  // namespace

LanguageRegistry::LanguageRegistry() = default;

void LanguageRegistry::loadBuiltinGrammars() {
  for (const auto& entry : grammar_data::kGrammars) {
    const TSLanguage* language = entry.languageFn();
    if (language == nullptr) {
      continue;
    }

    uint32_t abi = ts_language_abi_version(language);
    if (abi < TREE_SITTER_MIN_COMPATIBLE_LANGUAGE_VERSION || abi > TREE_SITTER_LANGUAGE_VERSION) {
      continue;
    }

    GrammarInfo info;
    info.name = entry.name;
    info.highlightsQuery = entry.highlightsQuery;
    info.language = language;

    const size_t index = grammars_.size();
    nameMap_[info.name] = index;
    grammars_.push_back(std::move(info));
  }

  for (const auto& ext : kExtensionTable) {
    auto it = nameMap_.find(ext.grammarName);
    if (it != nameMap_.end()) {
      extensionMap_[ext.extension] = it->second;
    }
  }
}

const GrammarInfo* LanguageRegistry::grammarForExtension(std::string_view extension) const {
  std::string ext(extension);
  auto it = extensionMap_.find(ext);
  if (it == extensionMap_.end()) {
    return nullptr;
  }
  return &grammars_.at(it->second);
}

const GrammarInfo* LanguageRegistry::grammarForName(std::string_view name) const {
  std::string n(name);
  auto it = nameMap_.find(n);
  if (it == nameMap_.end()) {
    return nullptr;
  }
  return &grammars_.at(it->second);
}

}  // namespace diffy
