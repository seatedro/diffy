use std::collections::HashMap;

use tree_sitter::Language;

#[derive(Debug, Clone)]
pub struct Grammar {
    pub name: &'static str,
    pub language: Language,
    pub highlights_query: &'static str,
}

#[derive(Debug, Default, Clone)]
pub struct LanguageRegistry {
    by_extension: HashMap<&'static str, Grammar>,
}

impl LanguageRegistry {
    pub fn new() -> Self {
        let mut registry = Self::default();
        registry.register_many();
        registry
    }

    pub fn grammar_for_extension(&self, extension: &str) -> Option<&Grammar> {
        self.by_extension.get(extension)
    }

    fn register_many(&mut self) {
        self.register(
            &[".sh", ".bash", ".zsh"],
            "bash",
            tree_sitter_bash::LANGUAGE.into(),
            tree_sitter_bash::HIGHLIGHT_QUERY,
        );
        self.register(
            &[".c", ".h"],
            "c",
            tree_sitter_c::LANGUAGE.into(),
            tree_sitter_c::HIGHLIGHT_QUERY,
        );
        self.register(
            &[".cc", ".cpp", ".cxx", ".hh", ".hpp", ".hxx"],
            "cpp",
            tree_sitter_cpp::LANGUAGE.into(),
            tree_sitter_cpp::HIGHLIGHT_QUERY,
        );
        self.register(
            &[".go"],
            "go",
            tree_sitter_go::LANGUAGE.into(),
            tree_sitter_go::HIGHLIGHTS_QUERY,
        );
        self.register(
            &[".js", ".jsx", ".mjs"],
            "javascript",
            tree_sitter_javascript::LANGUAGE.into(),
            tree_sitter_javascript::HIGHLIGHT_QUERY,
        );
        self.register(
            &[".json"],
            "json",
            tree_sitter_json::LANGUAGE.into(),
            tree_sitter_json::HIGHLIGHTS_QUERY,
        );
        self.register(
            &[".nix"],
            "nix",
            tree_sitter_nix::LANGUAGE.into(),
            tree_sitter_nix::HIGHLIGHTS_QUERY,
        );
        self.register(
            &[".py", ".pyi"],
            "python",
            tree_sitter_python::LANGUAGE.into(),
            tree_sitter_python::HIGHLIGHTS_QUERY,
        );
        self.register(
            &[".rs"],
            "rust",
            tree_sitter_rust::LANGUAGE.into(),
            tree_sitter_rust::HIGHLIGHTS_QUERY,
        );
        // toml and zig use different tree-sitter versions, skip for now
        self.register(
            &[".zig"],
            "zig",
            tree_sitter_zig::LANGUAGE.into(),
            tree_sitter_zig::HIGHLIGHTS_QUERY,
        );
    }

    fn register(
        &mut self,
        extensions: &[&'static str],
        name: &'static str,
        language: Language,
        highlights_query: &'static str,
    ) {
        let grammar = Grammar {
            name,
            language,
            highlights_query,
        };
        for extension in extensions {
            self.by_extension.insert(extension, grammar.clone());
        }
    }
}
