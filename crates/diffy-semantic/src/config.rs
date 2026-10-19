#[derive(Debug)]
pub struct Config {
    pub atoms: &'static [&'static str],
    pub delimiters: &'static [(&'static str, &'static str)],
}

const PAREN: (&str, &str) = ("(", ")");
const BRACE: (&str, &str) = ("{", "}");
const BRACKET: (&str, &str) = ("[", "]");
const ANGLE: (&str, &str) = ("<", ">");
const PIPE: (&str, &str) = ("|", "|");

const BASH: Config = Config {
    atoms: &["string", "raw_string", "heredoc_body", "simple_expansion"],
    delimiters: &[PAREN, BRACE, BRACKET],
};
const C: Config = Config {
    atoms: &["string_literal", "char_literal"],
    delimiters: &[PAREN, BRACE, BRACKET],
};
const CPP: Config = Config {
    atoms: &["string_literal", "char_literal"],
    delimiters: &[PAREN, BRACE, BRACKET, ANGLE],
};
const GO: Config = Config {
    atoms: &["interpreted_string_literal", "raw_string_literal"],
    delimiters: &[BRACE, BRACKET, PAREN],
};
const JAVASCRIPT: Config = Config {
    atoms: &["string", "template_string", "regex"],
    delimiters: &[BRACKET, PAREN, BRACE, ANGLE],
};
const JSON: Config = Config {
    atoms: &["string"],
    delimiters: &[BRACE, BRACKET],
};
const NIX: Config = Config {
    atoms: &["string_expression", "indented_string_expression"],
    delimiters: &[BRACE, BRACKET],
};
const PYTHON: Config = Config {
    atoms: &["string"],
    delimiters: &[PAREN, BRACKET, BRACE],
};
const RUST: Config = Config {
    atoms: &["char_literal", "string_literal", "raw_string_literal"],
    delimiters: &[BRACE, PAREN, BRACKET, PIPE, ANGLE],
};
const TOML: Config = Config {
    atoms: &["string", "quoted_key"],
    delimiters: &[BRACE, BRACKET],
};
const TSX: Config = Config {
    atoms: &["string", "template_string"],
    delimiters: &[BRACE, PAREN, BRACKET, ANGLE],
};
const TYPESCRIPT: Config = Config {
    atoms: &["string", "template_string", "regex", "predefined_type"],
    delimiters: &[BRACE, PAREN, BRACKET, ANGLE],
};

pub fn config(language: &str) -> Option<&'static Config> {
    Some(match language {
        "bash" => &BASH,
        "c" => &C,
        "cpp" => &CPP,
        "go" => &GO,
        "javascript" => &JAVASCRIPT,
        "json" => &JSON,
        "nix" => &NIX,
        "python" => &PYTHON,
        "rust" => &RUST,
        "toml" => &TOML,
        "tsx" => &TSX,
        "typescript" => &TYPESCRIPT,
        _ => return None,
    })
}
