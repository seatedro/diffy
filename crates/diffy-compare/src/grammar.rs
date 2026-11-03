//! Grammars for the semantic diff, from the caller's store.

use std::sync::{Arc, Mutex, PoisonError};

use diffy_semantic::{Queries, Rules};
use diffy_syntax::{GrammarQueries, GrammarStore, Injector, LanguageId};

/// What the semantic diff needs from one language's grammar.
pub struct SemanticGrammar {
    pub name: Arc<str>,
    pub language: diffy_semantic::Language,
    pub rules: Arc<Rules>,
    pub injector: Option<Injector>,
}

/// The grammar `store` has for `tag`, with its semantic rules compiled
/// once per loaded grammar.
pub fn semantic_grammar(store: &GrammarStore, tag: &LanguageId) -> Option<SemanticGrammar> {
    static RULES: Mutex<Vec<(Arc<str>, Arc<Rules>)>> = Mutex::new(Vec::new());
    let (language, queries) = store.parser(tag.as_str())?;
    let cached = |rules: &[(Arc<str>, Arc<Rules>)]| {
        rules
            .iter()
            .find(|(highlights, _)| Arc::ptr_eq(highlights, &queries.highlights))
            .map(|(_, rules)| rules.clone())
    };
    let found = cached(&RULES.lock().unwrap_or_else(PoisonError::into_inner));
    let rules = match found {
        Some(rules) => rules,
        None => {
            let rules = Arc::new(compile_rules(&language, &queries));
            let mut all = RULES.lock().unwrap_or_else(PoisonError::into_inner);
            match cached(&all) {
                Some(rules) => rules,
                None => {
                    all.push((queries.highlights.clone(), rules.clone()));
                    rules
                }
            }
        }
    };
    Some(SemanticGrammar {
        injector: store.injector(tag.as_str()),
        name: queries.language,
        language,
        rules,
    })
}

fn compile_rules(language: &diffy_semantic::Language, grammar: &GrammarQueries) -> Rules {
    let queries = Queries {
        highlights: &grammar.highlights,
        locals: grammar.locals.as_deref(),
    };
    let own = [
        diffy_semantic::semantic_query(&grammar.language),
        grammar.semantic.as_deref(),
    ];
    for semantic in own.into_iter().flatten() {
        match Rules::new(language, semantic, &queries) {
            Ok(rules) => return rules,
            Err(error) => tracing::warn!(
                target: "diffy::semantic",
                language = %grammar.language,
                %error,
                "semantic query does not compile"
            ),
        }
    }
    Rules::derived(language, &queries)
}
