use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

pub const CORE: &str = include_str!("../grammars.toml");
pub const REGISTRY: &str = include_str!("../registry.toml");

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub repo: String,
    pub rev: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Grammar {
    pub repo: String,
    pub rev: String,
    pub version: String,
    #[serde(default = "default_src", skip_serializing_if = "is_default_src")]
    pub src: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queries: Option<Source>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
    pub highlights: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub injections: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locals: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    pub sha256: String,
    /// SPDX expression for the grammar and its queries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    pub scanner: Scanner,
    #[serde(default, skip_serializing_if = "is_false")]
    pub generate: bool,
    /// Representative source, relative to this crate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample: Option<String>,
    /// `[text, capture]`: the first occurrence of `text` in the sample must
    /// be exactly the range of a capture named `capture` or `capture.*`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub captures: Vec<(String, String)>,
    /// Languages the injection query must name for the sample.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub injects: Vec<String>,
}

/// The external scanner a grammar's `src` holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Scanner {
    /// `scanner.c`, compiled beside `parser.c`.
    C,
    Cc,
    None,
}

fn default_src() -> String {
    "src".to_owned()
}

fn is_default_src(src: &String) -> bool {
    src == "src"
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    pub registry: Option<RegistryRef>,
    #[serde(default)]
    pub language: BTreeMap<String, Grammar>,
    #[serde(default)]
    pub fragment: BTreeMap<String, Source>,
    #[serde(default)]
    pub skipped: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryRef {
    pub url: String,
    pub rev: String,
}

pub struct Lock {
    pub grammars: BTreeMap<String, Grammar>,
    pub fragments: BTreeMap<String, Source>,
    pub core: BTreeSet<String>,
}

impl Lock {
    pub fn get(&self, language: &str) -> Option<&Grammar> {
        self.grammars.get(language)
    }

    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.grammars.keys()
    }

    pub fn is_core(&self, language: &str) -> bool {
        self.core.contains(language)
    }

    pub fn queries_of(&self, name: &str) -> Option<Source> {
        if let Some(fragment) = self.fragments.get(name) {
            return Some(fragment.clone());
        }
        let grammar = self.grammars.get(name)?;
        Some(grammar.queries.clone().unwrap_or(Source {
            repo: grammar.repo.clone(),
            rev: grammar.rev.clone(),
        }))
    }
}

pub fn load() -> Result<Lock, String> {
    parse(CORE, REGISTRY)
}

pub fn parse(core: &str, registry: &str) -> Result<Lock, String> {
    let core: BTreeMap<String, Grammar> =
        toml::from_str(core).map_err(|e| format!("grammars.toml: {e}"))?;
    let registry: Registry = toml::from_str(registry).map_err(|e| format!("registry.toml: {e}"))?;
    for (language, grammar) in &core {
        if grammar.captures.is_empty() || grammar.sample.is_none() {
            return Err(format!("{language}: no sample and expected captures"));
        }
        if grammar.license.is_none() {
            return Err(format!("{language}: no license"));
        }
    }
    let names: BTreeSet<String> = core.keys().cloned().collect();
    let mut grammars = core;
    for (language, grammar) in registry.language {
        if grammars.contains_key(&language) {
            return Err(format!(
                "{language} is in both grammars.toml and registry.toml"
            ));
        }
        grammars.insert(language, grammar);
    }
    let lock = Lock {
        grammars,
        fragments: registry.fragment,
        core: names,
    };
    validate(&lock)?;
    Ok(lock)
}

/// Checks what the build cannot: every name, alias, and extension means
/// one language (a store resolves a tag to whichever pack claims it
/// first), tags are lowercase as fence lookups are, and each expected
/// injection reaches a pinned pack.
fn validate(lock: &Lock) -> Result<(), String> {
    let mut owners: BTreeMap<&str, &str> = BTreeMap::new();
    for (language, grammar) in &lock.grammars {
        let tags = std::iter::once(language)
            .chain(&grammar.aliases)
            .chain(&grammar.extensions);
        for tag in tags {
            if *tag != tag.to_ascii_lowercase() {
                return Err(format!("{language}: tag {tag:?} is not lowercase"));
            }
            if diffy_syntax::LanguageId::from_fence(tag).is_none() {
                return Err(format!(
                    "{language}: tag {tag:?} is not a valid language tag"
                ));
            }
            if let Some(owner) = owners.insert(tag, language)
                && owner != language
            {
                return Err(format!("{tag:?} is claimed by both {owner} and {language}"));
            }
        }
    }
    for (language, grammar) in &lock.grammars {
        if let Some(missing) = grammar
            .injects
            .iter()
            .find(|tag| !owners.contains_key(tag.as_str()))
        {
            return Err(format!(
                "{language}: expected injection {missing:?} is no pinned language"
            ));
        }
    }
    Ok(())
}
