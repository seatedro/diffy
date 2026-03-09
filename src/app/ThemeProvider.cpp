#include "app/ThemeProvider.h"

#include <QFile>

#include <simdjson.h>

namespace diffy {
namespace {

const QStringList& modeNames() {
  static const QStringList kModeNames = {"dark", "light"};
  return kModeNames;
}

QString normalizeThemeName(const QString& name) {
  if (name == "gruvbox-dark" || name == "gruvbox-light") {
    return QStringLiteral("gruvbox");
  }
  if (name == "kanagawa-dark" || name == "kanagawa-light") {
    return QStringLiteral("kanagawa");
  }
  if (name == "rose-pine-dark" || name == "rose-pine-light") {
    return QStringLiteral("rose-pine");
  }
  if (name == "catppuccin-dark" || name == "catppuccin-light") {
    return QStringLiteral("catppuccin");
  }
  return name;
}

QString inferModeFromThemeValue(const QString& themeValue) {
  if (themeValue.endsWith("-light") || themeValue == "gruvbox-light") {
    return QStringLiteral("light");
  }
  if (themeValue.endsWith("-dark") || themeValue == "gruvbox-dark") {
    return QStringLiteral("dark");
  }
  return QString();
}

QString normalizeModeName(const QString& mode) {
  if (mode == "light") {
    return QStringLiteral("light");
  }
  if (mode == "dark") {
    return QStringLiteral("dark");
  }
  return QString();
}

QColor parseColor(const simdjson::dom::object& obj, const char* key) {
  std::string_view value;
  if (obj.at_key(key).get_string().get(value) != simdjson::SUCCESS) {
    return {};
  }
  return QColor(QString::fromUtf8(value.data(), static_cast<int>(value.size())));
}

ThemeColors parseThemeColors(const simdjson::dom::object& obj) {
  return {
      parseColor(obj, "appBg"),       parseColor(obj, "canvas"),        parseColor(obj, "panel"),
      parseColor(obj, "panelStrong"), parseColor(obj, "panelTint"),     parseColor(obj, "toolbarBg"),

      parseColor(obj, "borderSoft"),  parseColor(obj, "borderStrong"),  parseColor(obj, "divider"),

      parseColor(obj, "textStrong"),  parseColor(obj, "textBase"),      parseColor(obj, "textMuted"),
      parseColor(obj, "textFaint"),

      parseColor(obj, "accent"),      parseColor(obj, "accentStrong"),  parseColor(obj, "accentSoft"),

      parseColor(obj, "successBg"),   parseColor(obj, "successBorder"), parseColor(obj, "successText"),

      parseColor(obj, "dangerBg"),    parseColor(obj, "dangerBorder"),  parseColor(obj, "dangerText"),

      parseColor(obj, "warningBg"),   parseColor(obj, "warningBorder"), parseColor(obj, "warningText"),

      parseColor(obj, "selectionBg"), parseColor(obj, "selectionBorder"),

      parseColor(obj, "lineContext"), parseColor(obj, "lineContextAlt"), parseColor(obj, "lineAdd"),
      parseColor(obj, "lineAddAccent"), parseColor(obj, "lineDel"), parseColor(obj, "lineDelAccent"),

      parseColor(obj, "shadowSm"),    parseColor(obj, "shadowMd"),      parseColor(obj, "shadowLg")
  };
}

bool hasThemeNameInsensitive(const QStringList& names, const QString& candidate) {
  for (const QString& existing : names) {
    if (existing.compare(candidate, Qt::CaseInsensitive) == 0) {
      return true;
    }
  }
  return false;
}

}  // namespace

ThemeProvider::ThemeProvider(QObject* parent) : QObject(parent) {
  initializeThemes();

  const QString storedThemeRaw = settings_.value("theme", "gruvbox").toString();
  const QString storedModeRaw = settings_.value("themeMode", "").toString();

  QString resolvedTheme = normalizeThemeName(storedThemeRaw);
  if (!themes_.contains(resolvedTheme)) {
    for (const QString& candidate : themeNames_) {
      if (candidate.compare(resolvedTheme, Qt::CaseInsensitive) == 0) {
        resolvedTheme = candidate;
        break;
      }
    }
  }
  if (!themes_.contains(resolvedTheme)) {
    resolvedTheme = themes_.contains("gruvbox") ? QStringLiteral("gruvbox") : themeNames_.value(0);
  }

  QString resolvedMode = normalizeModeName(storedModeRaw);
  if (resolvedMode.isEmpty()) {
    resolvedMode = inferModeFromThemeValue(storedThemeRaw);
  }
  if (!modeNames().contains(resolvedMode)) {
    resolvedMode = QStringLiteral("dark");
  }

  currentTheme_ = resolvedTheme;
  currentMode_ = resolvedMode;
  loadTheme(currentTheme_, currentMode_);
}

void ThemeProvider::initializeThemes() {
  if (!themeNames_.isEmpty()) {
    return;
  }

  registerTheme("gruvbox", gruvboxDark(), gruvboxLight());
  registerTheme("kanagawa", kanagawaDark(), kanagawaLight());
  registerTheme("rose-pine", rosePineDark(), rosePineLight());
  registerTheme("catppuccin", catppuccinDark(), catppuccinLight());

  QFile f(QStringLiteral(":/themes/ghostty_themes.json"));
  if (!f.open(QIODevice::ReadOnly)) {
    return;
  }

  const QByteArray jsonBytes = f.readAll();
  simdjson::dom::parser parser;
  simdjson::padded_string padded(std::string_view(jsonBytes.constData(), static_cast<size_t>(jsonBytes.size())));
  simdjson::dom::element root;
  if (parser.parse(padded).get(root) != simdjson::SUCCESS) {
    return;
  }

  simdjson::dom::object rootObj;
  if (root.get_object().get(rootObj) != simdjson::SUCCESS) {
    return;
  }

  simdjson::dom::array themes;
  if (rootObj.at_key("themes").get_array().get(themes) != simdjson::SUCCESS) {
    return;
  }

  for (simdjson::dom::element value : themes) {
    simdjson::dom::object obj;
    if (value.get_object().get(obj) != simdjson::SUCCESS) {
      continue;
    }

    std::string_view nameRaw;
    if (obj.at_key("name").get_string().get(nameRaw) != simdjson::SUCCESS) {
      continue;
    }
    const QString name = QString::fromUtf8(nameRaw.data(), static_cast<int>(nameRaw.size())).trimmed();
    if (name.isEmpty() || hasThemeNameInsensitive(themeNames_, name)) {
      continue;
    }

    simdjson::dom::object darkObj;
    simdjson::dom::object lightObj;
    if (obj.at_key("dark").get_object().get(darkObj) != simdjson::SUCCESS ||
        obj.at_key("light").get_object().get(lightObj) != simdjson::SUCCESS) {
      continue;
    }

    const ThemeColors dark = parseThemeColors(darkObj);
    const ThemeColors light = parseThemeColors(lightObj);
    if (!dark.appBg.isValid() || !light.appBg.isValid()) {
      continue;
    }

    registerTheme(name, dark, light);
  }
}

void ThemeProvider::registerTheme(const QString& name, const ThemeColors& dark, const ThemeColors& light) {
  if (themes_.contains(name)) {
    themes_[name] = {.dark = dark, .light = light};
    return;
  }

  themes_.insert(name, {.dark = dark, .light = light});
  themeNames_.append(name);
}

QString ThemeProvider::currentTheme() const {
  return currentTheme_;
}

QString ThemeProvider::currentMode() const {
  return currentMode_;
}

QStringList ThemeProvider::availableThemes() const {
  return themeNames_;
}

QStringList ThemeProvider::availableModes() const {
  return modeNames();
}

void ThemeProvider::setTheme(const QString& name, bool persist) {
  QString resolvedTheme = normalizeThemeName(name);

  if (!themes_.contains(resolvedTheme)) {
    for (const QString& candidate : themeNames_) {
      if (candidate.compare(resolvedTheme, Qt::CaseInsensitive) == 0) {
        resolvedTheme = candidate;
        break;
      }
    }
  }

  if (!themes_.contains(resolvedTheme)) {
    resolvedTheme = themes_.contains("gruvbox") ? QStringLiteral("gruvbox") : themeNames_.value(0);
  }

  if (resolvedTheme == currentTheme_) {
    if (persist) {
      settings_.setValue("theme", currentTheme_);
      settings_.setValue("themeMode", currentMode_);
    }
    return;
  }

  loadTheme(resolvedTheme, currentMode_);
  currentTheme_ = resolvedTheme;
  if (persist) {
    settings_.setValue("theme", currentTheme_);
    settings_.setValue("themeMode", currentMode_);
  }
  emit themeChanged();
}

void ThemeProvider::setMode(const QString& mode, bool persist) {
  QString resolvedMode = normalizeModeName(mode);
  if (!modeNames().contains(resolvedMode)) {
    resolvedMode = QStringLiteral("dark");
  }

  if (resolvedMode == currentMode_) {
    if (persist) {
      settings_.setValue("theme", currentTheme_);
      settings_.setValue("themeMode", currentMode_);
    }
    return;
  }

  loadTheme(currentTheme_, resolvedMode);
  currentMode_ = resolvedMode;
  if (persist) {
    settings_.setValue("theme", currentTheme_);
    settings_.setValue("themeMode", currentMode_);
  }
  emit themeChanged();
}

void ThemeProvider::toggleMode(bool persist) {
  setMode(currentMode_ == "dark" ? QStringLiteral("light") : QStringLiteral("dark"), persist);
}

void ThemeProvider::loadTheme(const QString& name, const QString& mode) {
  if (!themes_.contains(name)) {
    const QString fallback = themes_.contains("gruvbox") ? QStringLiteral("gruvbox") : themeNames_.value(0);
    const ThemeVariants fallbackVariants = themes_.value(fallback);
    colors_ = mode == "light" ? fallbackVariants.light : fallbackVariants.dark;
    return;
  }

  const ThemeVariants variants = themes_.value(name);
  colors_ = mode == "light" ? variants.light : variants.dark;
}

ThemeColors ThemeProvider::gruvboxDark() {
  return {
      QColor("#1d2021"), QColor("#282828"), QColor("#32302f"), QColor("#3c3836"), QColor("#504945"), QColor("#282828"),
      QColor("#504945"), QColor("#665c54"), QColor("#504945"),
      QColor("#fbf1c7"), QColor("#ebdbb2"), QColor("#d5c4a1"), QColor("#a89984"),
      QColor("#83a598"), QColor("#83a598"), QColor("#3b4b4f"),
      QColor("#32361a"), QColor("#4a5a1c"), QColor("#b8bb26"),
      QColor("#3c1f1e"), QColor("#7c3a31"), QColor("#fb4934"),
      QColor("#4a3b16"), QColor("#7c6f27"), QColor("#fabd2f"),
      QColor("#3c3836"), QColor("#83a598"),
      QColor("#282828"), QColor("#232323"), QColor("#2a3a1e"), QColor("#324420"), QColor("#3d2020"), QColor("#4c2828"),
      QColor("#1a000000"), QColor("#33000000"), QColor("#4d000000")
  };
}

ThemeColors ThemeProvider::gruvboxLight() {
  return {
      QColor("#fbf1c7"), QColor("#f2e5bc"), QColor("#ebdbb2"), QColor("#d5c4a1"), QColor("#bdae93"), QColor("#f2e5bc"),
      QColor("#d5c4a1"), QColor("#bdae93"), QColor("#d5c4a1"),
      QColor("#282828"), QColor("#3c3836"), QColor("#504945"), QColor("#928374"),
      QColor("#076678"), QColor("#076678"), QColor("#d5e5e8"),
      QColor("#e6ecc0"), QColor("#b0b846"), QColor("#79740e"),
      QColor("#f5d5c8"), QColor("#cc4416"), QColor("#9d0006"),
      QColor("#f5e6c4"), QColor("#d79920"), QColor("#b57614"),
      QColor("#ebdbb2"), QColor("#076678"),
      QColor("#f2e5bc"), QColor("#eddcaa"), QColor("#dde5c0"), QColor("#c8d6a0"), QColor("#f0d0c0"), QColor("#e5b8a8"),
      QColor("#0a000000"), QColor("#15000000"), QColor("#22000000")
  };
}

ThemeColors ThemeProvider::kanagawaDark() {
  return {
      QColor("#1F1F28"), QColor("#2A2A37"), QColor("#363646"), QColor("#54546D"), QColor("#223249"), QColor("#2A2A37"),
      QColor("#54546D"), QColor("#727169"), QColor("#54546D"),
      QColor("#DCD7BA"), QColor("#C8C093"), QColor("#C8C093"), QColor("#727169"),
      QColor("#7E9CD8"), QColor("#7E9CD8"), QColor("#2D4F67"),
      QColor("#2B3328"), QColor("#76946A"), QColor("#98BB6C"),
      QColor("#43242B"), QColor("#C34043"), QColor("#FF5D62"),
      QColor("#49443C"), QColor("#DCA561"), QColor("#E6C384"),
      QColor("#223249"), QColor("#7E9CD8"),
      QColor("#2A2A37"), QColor("#1a1a22"), QColor("#2B3328"), QColor("#76946A"), QColor("#43242B"), QColor("#C34043"),
      QColor("#1a000000"), QColor("#33000000"), QColor("#4d000000")
  };
}

ThemeColors ThemeProvider::kanagawaLight() {
  return {
      QColor("#f2ecbc"), QColor("#e5ddb0"), QColor("#e7dba0"), QColor("#e4d794"), QColor("#c9cbd1"), QColor("#e5ddb0"),
      QColor("#dfdad9"), QColor("#cecacd"), QColor("#dfdad9"),
      QColor("#545464"), QColor("#43436c"), QColor("#716e61"), QColor("#8a8980"),
      QColor("#4d699b"), QColor("#4d699b"), QColor("#c7d7e0"),
      QColor("#dfead8"), QColor("#6e915f"), QColor("#6f894e"),
      QColor("#f1d9d4"), QColor("#c84053"), QColor("#b35b79"),
      QColor("#f7e8c9"), QColor("#cc6d00"), QColor("#836f4a"),
      QColor("#c9cbd1"), QColor("#4d699b"),
      QColor("#e5ddb0"), QColor("#e4d794"), QColor("#b7d0ae"), QColor("#6e915f"), QColor("#d9a594"), QColor("#d7474b"),
      QColor("#0a000000"), QColor("#15000000"), QColor("#22000000")
  };
}

ThemeColors ThemeProvider::rosePineDark() {
  return {
      QColor("#191724"), QColor("#1f1d2e"), QColor("#26233a"), QColor("#403d52"), QColor("#524f67"), QColor("#1f1d2e"),
      QColor("#403d52"), QColor("#524f67"), QColor("#403d52"),
      QColor("#e0def4"), QColor("#e0def4"), QColor("#908caa"), QColor("#6e6a86"),
      QColor("#c4a7e7"), QColor("#c4a7e7"), QColor("#3a2e58"),
      QColor("#28303a"), QColor("#95b1ac"), QColor("#9ccfd8"),
      QColor("#3a2030"), QColor("#b4637a"), QColor("#eb6f92"),
      QColor("#3a3028"), QColor("#ea9d34"), QColor("#f6c177"),
      QColor("#26233a"), QColor("#c4a7e7"),
      QColor("#1f1d2e"), QColor("#21202e"), QColor("#263038"), QColor("#95b1ac"), QColor("#382028"), QColor("#eb6f92"),
      QColor("#1a000000"), QColor("#33000000"), QColor("#4d000000")
  };
}

ThemeColors ThemeProvider::rosePineLight() {
  return {
      QColor("#faf4ed"), QColor("#fffaf3"), QColor("#f2e9e1"), QColor("#dfdad9"), QColor("#cecacd"), QColor("#fffaf3"),
      QColor("#dfdad9"), QColor("#cecacd"), QColor("#dfdad9"),
      QColor("#464261"), QColor("#575279"), QColor("#797593"), QColor("#9893a5"),
      QColor("#907aa9"), QColor("#907aa9"), QColor("#ece3f6"),
      QColor("#e5f0ef"), QColor("#6d8f89"), QColor("#56949f"),
      QColor("#f2dde4"), QColor("#d7827e"), QColor("#b4637a"),
      QColor("#f7ead8"), QColor("#ea9d34"), QColor("#ea9d34"),
      QColor("#f2e9e1"), QColor("#907aa9"),
      QColor("#fffaf3"), QColor("#f4ede8"), QColor("#e7efea"), QColor("#6d8f89"), QColor("#f2e1e5"), QColor("#b4637a"),
      QColor("#0a000000"), QColor("#15000000"), QColor("#22000000")
  };
}

ThemeColors ThemeProvider::catppuccinDark() {
  return {
      QColor("#1e1e2e"), QColor("#24243e"), QColor("#313244"), QColor("#45475a"), QColor("#585b70"), QColor("#24243e"),
      QColor("#45475a"), QColor("#585b70"), QColor("#45475a"),
      QColor("#cdd6f4"), QColor("#bac2de"), QColor("#a6adc8"), QColor("#6c7086"),
      QColor("#89b4fa"), QColor("#89b4fa"), QColor("#2a3a5a"),
      QColor("#283028"), QColor("#406040"), QColor("#a6e3a1"),
      QColor("#382028"), QColor("#6a3040"), QColor("#f38ba8"),
      QColor("#383020"), QColor("#6a5830"), QColor("#f9e2af"),
      QColor("#313244"), QColor("#89b4fa"),
      QColor("#24243e"), QColor("#1e1e30"), QColor("#263028"), QColor("#2e3a30"), QColor("#382028"), QColor("#402830"),
      QColor("#1a000000"), QColor("#33000000"), QColor("#4d000000")
  };
}

ThemeColors ThemeProvider::catppuccinLight() {
  return {
      QColor("#eff1f5"), QColor("#e6e9ef"), QColor("#dce0e8"), QColor("#ccd0da"), QColor("#bcc0cc"), QColor("#e6e9ef"),
      QColor("#ccd0da"), QColor("#bcc0cc"), QColor("#ccd0da"),
      QColor("#4c4f69"), QColor("#5c5f77"), QColor("#6c6f85"), QColor("#8c8fa1"),
      QColor("#1e66f5"), QColor("#1e66f5"), QColor("#dbe7fd"),
      QColor("#e1efde"), QColor("#8fc485"), QColor("#40a02b"),
      QColor("#f4dce1"), QColor("#e08aa1"), QColor("#d20f39"),
      QColor("#f6ecd9"), QColor("#e6b36a"), QColor("#df8e1d"),
      QColor("#ccd0da"), QColor("#1e66f5"),
      QColor("#e6e9ef"), QColor("#dde0e6"), QColor("#e2efdf"), QColor("#d4e7d0"), QColor("#f3dde1"), QColor("#ebccd3"),
      QColor("#0a000000"), QColor("#15000000"), QColor("#22000000")
  };
}

}  // namespace diffy
