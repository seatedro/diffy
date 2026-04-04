use crate::ui::devicons::devicon;
use crate::ui::element::{svg_icon, AnyElement, ElementContext, IntoAnyElement, RenderOnce};
use crate::ui::icons::lucide;
use crate::ui::theme::Color;

#[derive(Clone, Copy)]
struct LangDef {
    icon: &'static str,
    hue: LangHue,
}

#[derive(Clone, Copy)]
pub enum LangHue {
    Orange,
    Blue,
    Yellow,
    Green,
    Red,
    Purple,
    Cyan,
    Pink,
    Muted,
}

fn lang_color(hue: LangHue, theme_muted: Color) -> Color {
    match hue {
        LangHue::Orange => Color::rgba(227, 134, 53, 255),
        LangHue::Blue => Color::rgba(66, 135, 245, 255),
        LangHue::Yellow => Color::rgba(227, 200, 60, 255),
        LangHue::Green => Color::rgba(80, 190, 100, 255),
        LangHue::Red => Color::rgba(220, 75, 65, 255),
        LangHue::Purple => Color::rgba(160, 100, 230, 255),
        LangHue::Cyan => Color::rgba(60, 195, 210, 255),
        LangHue::Pink => Color::rgba(220, 100, 160, 255),
        LangHue::Muted => theme_muted,
    }
}

fn lookup_ext(ext: &str) -> Option<LangDef> {
    Some(match ext {
        "rs" => LangDef { icon: devicon::RUST, hue: LangHue::Orange },
        "ts" | "mts" | "cts" => LangDef { icon: devicon::TYPESCRIPT, hue: LangHue::Blue },
        "tsx" => LangDef { icon: devicon::REACT, hue: LangHue::Blue },
        "js" | "mjs" | "cjs" => LangDef { icon: devicon::JAVASCRIPT, hue: LangHue::Yellow },
        "jsx" => LangDef { icon: devicon::REACT, hue: LangHue::Cyan },
        "py" | "pyi" | "pyw" => LangDef { icon: devicon::PYTHON, hue: LangHue::Green },
        "go" => LangDef { icon: devicon::GO, hue: LangHue::Cyan },
        "java" => LangDef { icon: devicon::JAVA, hue: LangHue::Red },
        "c" | "h" => LangDef { icon: devicon::C_LANG, hue: LangHue::Blue },
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" | "hh" => LangDef { icon: devicon::CPP, hue: LangHue::Blue },
        "cs" => LangDef { icon: devicon::CSHARP, hue: LangHue::Purple },
        "rb" | "rake" | "gemspec" => LangDef { icon: devicon::RUBY, hue: LangHue::Red },
        "swift" => LangDef { icon: devicon::SWIFT, hue: LangHue::Orange },
        "kt" | "kts" => LangDef { icon: devicon::KOTLIN, hue: LangHue::Purple },
        "php" => LangDef { icon: devicon::PHP, hue: LangHue::Purple },
        "html" | "htm" => LangDef { icon: devicon::HTML, hue: LangHue::Orange },
        "css" | "scss" | "sass" | "less" => LangDef { icon: devicon::CSS, hue: LangHue::Blue },
        "vue" => LangDef { icon: devicon::VUE, hue: LangHue::Green },
        "svelte" => LangDef { icon: devicon::SVELTE, hue: LangHue::Orange },
        "dart" => LangDef { icon: devicon::DART, hue: LangHue::Cyan },
        "lua" => LangDef { icon: devicon::LUA, hue: LangHue::Blue },
        "pl" | "pm" => LangDef { icon: devicon::PERL, hue: LangHue::Muted },
        "scala" | "sc" => LangDef { icon: devicon::SCALA, hue: LangHue::Red },
        "hs" | "lhs" => LangDef { icon: devicon::HASKELL, hue: LangHue::Purple },
        "ex" | "exs" => LangDef { icon: devicon::ELIXIR, hue: LangHue::Purple },
        "sh" | "zsh" | "fish" => LangDef { icon: devicon::BASH, hue: LangHue::Muted },
        "yml" | "yaml" => LangDef { icon: devicon::YAML, hue: LangHue::Pink },
        "md" | "mdx" | "rst" => LangDef { icon: devicon::MARKDOWN, hue: LangHue::Blue },
        _ => return None,
    })
}

fn lookup_filename(name: &str) -> Option<LangDef> {
    let lower = name.to_lowercase();
    Some(match lower.as_str() {
        "dockerfile" | "containerfile" => LangDef { icon: devicon::DOCKER, hue: LangHue::Blue },
        "makefile" | "gnumakefile" => LangDef { icon: devicon::BASH, hue: LangHue::Muted },
        "cargo.toml" | "cargo.lock" => LangDef { icon: devicon::RUST, hue: LangHue::Orange },
        "package.json" | "package-lock.json" => LangDef { icon: devicon::JAVASCRIPT, hue: LangHue::Yellow },
        "tsconfig.json" => LangDef { icon: devicon::TYPESCRIPT, hue: LangHue::Blue },
        "gemfile" | "rakefile" => LangDef { icon: devicon::RUBY, hue: LangHue::Red },
        "go.mod" | "go.sum" => LangDef { icon: devicon::GO, hue: LangHue::Cyan },
        ".gitignore" | ".gitattributes" | ".gitmodules" => LangDef { icon: lucide::GIT_BRANCH, hue: LangHue::Orange },
        _ => return None,
    })
}

pub fn resolve_file_icon(path: &str) -> (&'static str, LangHue) {
    let filename = path.rsplit('/').next().unwrap_or(path);

    if let Some(def) = lookup_filename(filename) {
        return (def.icon, def.hue);
    }

    if let Some(ext) = filename.rsplit('.').next() {
        if ext != filename {
            if let Some(def) = lookup_ext(ext) {
                return (def.icon, def.hue);
            }
        }
    }

    (lucide::FILE, LangHue::Muted)
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
        let (icon_svg, hue) = resolve_file_icon(&self.path);

        let color = if self.selected {
            tc.accent
        } else {
            lang_color(hue, tc.text_muted)
        };

        svg_icon(icon_svg, self.size).color(color).into_any()
    }
}
