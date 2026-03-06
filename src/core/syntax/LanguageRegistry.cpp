#include "core/syntax/LanguageRegistry.h"

#include <dlfcn.h>

#include <filesystem>
#include <fstream>
#include <sstream>

#include <tree_sitter/api.h>

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

std::string readFileContents(const std::filesystem::path& path) {
  std::ifstream file(path);
  if (!file.is_open()) {
    return {};
  }
  std::ostringstream ss;
  ss << file.rdbuf();
  return ss.str();
}

}  // namespace

LanguageRegistry::LanguageRegistry() = default;

LanguageRegistry::~LanguageRegistry() {
  for (auto& grammar : grammars_) {
    if (grammar.dlHandle != nullptr) {
      dlclose(grammar.dlHandle);
      grammar.dlHandle = nullptr;
    }
  }
}

void LanguageRegistry::discoverGrammars(const std::string& searchPaths) {
  std::istringstream stream(searchPaths);
  std::string entry;

  while (std::getline(stream, entry, ':')) {
    if (entry.empty()) {
      continue;
    }

    namespace fs = std::filesystem;
    const fs::path dir(entry);
    if (!fs::is_directory(dir)) {
      continue;
    }

    const fs::path parserFile = dir / "parser";
    if (!fs::is_regular_file(parserFile)) {
      continue;
    }

    std::string dirName = dir.filename().string();
    const std::string prefix = "tree-sitter-";
    std::string grammarName;
    if (dirName.starts_with(prefix)) {
      grammarName = dirName.substr(prefix.size());
    } else {
      auto dashPos = dirName.find('-');
      if (dashPos != std::string::npos) {
        grammarName = dirName.substr(dashPos + 1);
      } else {
        grammarName = dirName;
      }
    }

    std::string highlightsQuery;
    const fs::path queriesDir = dir / "queries";
    if (fs::is_directory(queriesDir)) {
      highlightsQuery = readFileContents(queriesDir / "highlights.scm");
    }

    GrammarInfo info;
    info.name = grammarName;
    info.parserPath = parserFile.string();
    info.highlightsQuery = std::move(highlightsQuery);

    if (loadGrammar(info)) {
      const size_t index = grammars_.size();
      nameMap_[info.name] = index;
      grammars_.push_back(std::move(info));
    }
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

bool LanguageRegistry::loadGrammar(GrammarInfo& info) {
  void* handle = dlopen(info.parserPath.c_str(), RTLD_LAZY | RTLD_LOCAL);
  if (handle == nullptr) {
    return false;
  }

  std::string symbolName = "tree_sitter_" + info.name;
  for (char& ch : symbolName) {
    if (ch == '-') {
      ch = '_';
    }
  }

  using LanguageFn = const TSLanguage* (*)();
  auto langFn = reinterpret_cast<LanguageFn>(dlsym(handle, symbolName.c_str()));
  if (langFn == nullptr) {
    dlclose(handle);
    return false;
  }

  const TSLanguage* language = langFn();
  if (language == nullptr) {
    dlclose(handle);
    return false;
  }

  uint32_t abi = ts_language_abi_version(language);
  if (abi < TREE_SITTER_MIN_COMPATIBLE_LANGUAGE_VERSION || abi > TREE_SITTER_LANGUAGE_VERSION) {
    dlclose(handle);
    return false;
  }

  info.language = language;
  info.dlHandle = handle;
  return true;
}

}  // namespace diffy
