use halogen::view;

use crate::ui::element::{AnyElement, ElementContext, IntoAnyElement, RenderOnce, svg_icon};
use crate::ui::symbols::symbols as sym;

fn lookup_ext(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "rs" => sym::RUST,
        "ts" | "mts" | "cts" => sym::TS,
        "tsx" => sym::REACT_TS,
        "js" | "mjs" | "cjs" => sym::JS,
        "jsx" => sym::REACT,
        "py" | "pyi" | "pyw" => sym::PYTHON,
        "go" => sym::GO,
        "java" => sym::JAVA,
        "c" => sym::C,
        "h" => sym::H,
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" | "hh" => sym::CPLUS,
        "cs" => sym::CSHARP,
        "rb" | "rake" | "gemspec" => sym::RUBY,
        "swift" => sym::SWIFT,
        "kt" | "kts" => sym::KOTLIN,
        "php" => sym::PHP,
        "html" | "htm" => sym::CODE_ORANGE,
        "css" => sym::CODE_BLUE,
        "scss" | "sass" => sym::SASS,
        "less" => sym::CODE_BLUE,
        "vue" => sym::VUE,
        "svelte" => sym::SVELTE,
        "dart" => sym::DART,
        "lua" | "luau" => sym::LUA,
        "pl" | "pm" => sym::PERL,
        "scala" | "sc" => sym::SCALA,
        "hs" | "lhs" => sym::HASKELL,
        "ex" | "exs" => sym::ELIXIR,
        "erl" | "hrl" => sym::ERLANG,
        "elm" => sym::CODE_GREEN,
        "sh" | "zsh" | "fish" | "bash" => sym::SHELL,
        "yml" | "yaml" => sym::YAML,
        "md" | "mdx" => sym::MARKDOWN,
        "json" | "jsonc" => sym::BRACKETS_YELLOW,
        "toml" => sym::BRACKETS_GRAY,
        "xml" | "plist" | "xsl" | "xslt" => sym::XML,
        "graphql" | "gql" => sym::GRAPHQL,
        "r" | "rmd" => sym::R,
        "nix" => sym::NIX,
        "tf" | "tfvars" => sym::TERRAFORM,
        "prisma" => sym::PRISMA,
        "vim" => sym::CODE_GREEN,
        "sql" | "db" | "sqlite" => sym::DATABASE,
        "lock" => sym::LOCK,
        "csv" | "tsv" => sym::CSV,
        "zig" => sym::ZIG,
        "clj" | "cljs" | "cljc" => sym::CLOJURE,
        "fs" | "fsx" | "fsi" => sym::FSHARP,
        "jl" => sym::JULIA,
        "nim" => sym::NIM,
        "ocaml" | "ml" | "mli" => sym::OCAML,
        "sol" => sym::SOLIDITY,
        "tex" | "latex" => sym::TEX,
        "proto" => sym::PROTO,
        "astro" => sym::ASTRO,
        "svx" => sym::SVX,
        "pug" => sym::PUG,
        "haml" => sym::HAML,
        "styl" => sym::STYLUS,
        "coffee" => sym::COFFEESCRIPT,
        "v" => sym::V,
        "cr" => sym::CRYSTAL,
        "pkl" => sym::PKL,
        "re" | "rei" => sym::RESCRIPT,
        "razor" | "cshtml" => sym::RAZOR,
        "liquid" => sym::LIQUID,
        "twig" => sym::TWIG,
        "svg" => sym::SVG,
        "png" | "jpg" | "jpeg" | "webp" | "ico" | "bmp" | "tiff" => sym::IMAGE,
        "gif" => sym::GIF,
        "mp4" | "mov" | "avi" | "mkv" | "webm" => sym::VIDEO,
        "mp3" | "wav" | "ogg" | "flac" | "aac" => sym::AUDIO,
        "pdf" => sym::PDF,
        "woff" | "woff2" | "ttf" | "otf" | "eot" => sym::FONT,
        "exe" | "dll" | "so" | "dylib" => sym::EXE,
        "patch" | "diff" => sym::PATCH,
        "rs.bk" | "bak" | "orig" => sym::DOCUMENT,
        "txt" | "text" => sym::TEXT,
        "ipynb" => sym::NOTEBOOK,
        "http" | "rest" => sym::HTTP,
        "drawio" => sym::DRAWIO,
        "gradle" | "gradle.kts" => sym::GRADLE,
        "sbt" => sym::SBT,
        "cmake" => sym::CMAKE,
        _ => return None,
    })
}

fn lookup_filename(name: &str) -> Option<&'static str> {
    let lower = name.to_lowercase();
    Some(match lower.as_str() {
        "dockerfile" | "containerfile" => sym::DOCKER,
        "makefile" | "gnumakefile" | "justfile" => sym::SHELL,
        "cargo.toml" | "cargo.lock" => sym::RUST,
        "package.json" | "package-lock.json" => sym::NPM,
        "tsconfig.json" | "tsconfig.base.json" => sym::TSCONFIG,
        "gemfile" | "rakefile" => sym::RUBY,
        "go.mod" | "go.sum" => sym::GO,
        ".gitignore" | ".gitattributes" | ".gitmodules" => sym::GIT,
        ".eslintrc" | ".eslintrc.js" | ".eslintrc.json" | "eslint.config.js"
        | "eslint.config.ts" | "eslint.config.mjs" => sym::ESLINT,
        ".prettierrc"
        | ".prettierrc.js"
        | ".prettierrc.json"
        | "prettier.config.js"
        | "prettier.config.mjs" => sym::PRETTIER,
        ".babelrc" | "babel.config.js" | "babel.config.json" => sym::BABEL,
        "docker-compose.yml" | "docker-compose.yaml" | "compose.yml" | "compose.yaml" => {
            sym::DOCKER
        }
        "yarn.lock" => sym::YARN,
        "pnpm-lock.yaml" | ".pnpmfile.cjs" => sym::PNPM,
        "bun.lockb" | "bunfig.toml" => sym::BUN,
        "next.config.js" | "next.config.ts" | "next.config.mjs" => sym::NEXT,
        "angular.json" => sym::ANGULAR,
        "flake.nix" | "flake.lock" => sym::NIX,
        "biome.json" | "biome.jsonc" => sym::BIOME,
        "turbo.json" => sym::TURBOREPO,
        ".editorconfig" => sym::EDITORCONFIG,
        "license" | "license.md" | "license.txt" | "licence" => sym::LICENSE,
        "vite.config.ts" | "vite.config.js" | "vite.config.mjs" => sym::VITE,
        "vitest.config.ts" | "vitest.config.js" => sym::VITEST,
        "webpack.config.js" | "webpack.config.ts" => sym::WEBPACK,
        "tailwind.config.js" | "tailwind.config.ts" | "tailwind.config.mjs" => sym::TAILWIND,
        "postcss.config.js" | "postcss.config.mjs" | "postcss.config.ts" => sym::POSTCSS,
        "jest.config.js" | "jest.config.ts" => sym::JEST,
        "cypress.config.js" | "cypress.config.ts" => sym::CYPRESS,
        ".storybook" | "stories.tsx" | "stories.jsx" => sym::STORYBOOK,
        "deno.json" | "deno.jsonc" | "deno.lock" => sym::DENO,
        "vercel.json" => sym::VERCEL,
        "netlify.toml" => sym::NETLIFY,
        "firebase.json" | ".firebaserc" => sym::FIREBASE,
        "nuxt.config.ts" | "nuxt.config.js" => sym::NUXT,
        "gatsby-config.js" | "gatsby-config.ts" => sym::GATSBY,
        "svelte.config.js" | "svelte.config.ts" => sym::SVELTE,
        ".github" => sym::GITHUB,
        ".gitlab-ci.yml" => sym::GITLAB,
        "jenkinsfile" => sym::JENKINS,
        "nodemon.json" => sym::NODEMON,
        "nx.json" => sym::NX,
        "pulumi.yaml" | "pulumi.yml" => sym::PULUMI,
        ".swcrc" => sym::SWC,
        "tauri.conf.json" => sym::TAURI,
        "capacitor.config.ts" | "capacitor.config.json" => sym::CAPACITOR,
        "ionic.config.json" => sym::IONIC,
        "hugo.toml" | "hugo.yaml" | "hugo.json" => sym::HUGO,
        "sanity.config.ts" | "sanity.config.js" => sym::SANITY,
        "supabase" => sym::SUPABASE,
        "drizzle.config.ts" => sym::DRIZZLE,
        _ => return None,
    })
}

pub fn resolve_file_icon(path: &str) -> &'static str {
    let filename = path.rsplit('/').next().unwrap_or(path);

    if let Some(icon) = lookup_filename(filename) {
        return icon;
    }

    if let Some(ext) = filename.rsplit('.').next() {
        if ext != filename {
            if let Some(icon) = lookup_ext(ext) {
                return icon;
            }
        }
    }

    sym::DOCUMENT
}

pub struct FileIcon {
    path: String,
    size: f32,
    selected: bool,
}

pub fn file_icon(path: impl Into<String>, size: f32) -> FileIcon {
    FileIcon {
        path: path.into(),
        size,
        selected: false,
    }
}

impl FileIcon {
    pub fn selected(mut self, s: bool) -> Self {
        self.selected = s;
        self
    }
}

impl RenderOnce for FileIcon {
    fn render(self, cx: &ElementContext) -> AnyElement {
        let tc = &cx.theme.colors;
        let icon_svg = resolve_file_icon(&self.path);
        let color = if self.selected {
            tc.text
        } else {
            tc.text_muted
        };

        view! {
            <icon svg={icon_svg} size={self.size} color={color} />
        }
    }
}
