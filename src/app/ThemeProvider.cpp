#include "app/ThemeProvider.h"

#include <QFile>
#include <QtResource>

#include <simdjson.h>

void ensureThemeResourcesLoaded() {
  static const bool kInitialized = []() {
    Q_INIT_RESOURCE(ThemeProviderThemes);
    return true;
  }();
  Q_UNUSED(kInitialized);
}

namespace diffy {
namespace {

const QStringList& modeNames() {
  static const QStringList kModeNames = {"dark", "light"};
  return kModeNames;
}

QColor hexColor(const char* value) {
  return QColor(QString::fromLatin1(value));
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

ThemeColors builtInDarkThemeColors() {
  return {
      hexColor("#1b1e24"), hexColor("#20242b"), hexColor("#282d36"), hexColor("#323844"), hexColor("#2b4d6d"),
      hexColor("#20242b"),

      hexColor("#39414d"), hexColor("#4e5968"), hexColor("#39414d"),

      hexColor("#f2f5f8"), hexColor("#e2e7ec"), hexColor("#a9b3bf"), hexColor("#7f8894"),

      hexColor("#5da9f6"), hexColor("#8cc3ff"), hexColor("#23394d"),

      hexColor("#24352a"), hexColor("#335843"), hexColor("#7dd69c"),

      hexColor("#3a2728"), hexColor("#6a3f40"), hexColor("#f28b82"),

      hexColor("#3d3224"), hexColor("#705734"), hexColor("#f3c56c"),

      hexColor("#2f3b4f"), hexColor("#5da9f6"),

      hexColor("#20242b"), hexColor("#232831"), hexColor("#24342a"), hexColor("#2d4736"), hexColor("#382728"),
      hexColor("#4a3133"),

      hexColor("#1a000000"), hexColor("#33000000"), hexColor("#4d000000")
  };
}

ThemeColors builtInLightThemeColors() {
  return {
      hexColor("#f7f7f5"), hexColor("#fbfbfa"), hexColor("#f1f1ef"), hexColor("#e6e6e3"), hexColor("#d7e4f2"),
      hexColor("#fbfbfa"),

      hexColor("#d3d3cf"), hexColor("#b4b4af"), hexColor("#d3d3cf"),

      hexColor("#1e1f21"), hexColor("#2a2c2f"), hexColor("#5f646b"), hexColor("#858b93"),

      hexColor("#0f68a0"), hexColor("#0b4f79"), hexColor("#d8e7f1"),

      hexColor("#e8f1ea"), hexColor("#cfe2d4"), hexColor("#2f6f3e"),

      hexColor("#f6e8e6"), hexColor("#ecc9c4"), hexColor("#b63424"),

      hexColor("#f5efe1"), hexColor("#e7d6b5"), hexColor("#8a5a18"),

      hexColor("#dbe9f6"), hexColor("#0f68a0"),

      hexColor("#fbfbfa"), hexColor("#f7f7f5"), hexColor("#edf5ef"), hexColor("#d8eadf"), hexColor("#f9ecea"),
      hexColor("#f1d6d1"),

      hexColor("#0a000000"), hexColor("#15000000"), hexColor("#22000000")
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
  ensureThemeResourcesLoaded();
  initializeThemes();

  const QString fallbackTheme =
      themes_.contains(QStringLiteral("Diffy")) ? QStringLiteral("Diffy") : themeNames_.value(0);
  const QString storedThemeRaw = settings_.value("theme", fallbackTheme).toString();
  const QString storedModeRaw = settings_.value("themeMode", "").toString();

  QString resolvedTheme = storedThemeRaw.trimmed();
  if (!themes_.contains(resolvedTheme)) {
    for (const QString& candidate : themeNames_) {
      if (candidate.compare(resolvedTheme, Qt::CaseInsensitive) == 0) {
        resolvedTheme = candidate;
        break;
      }
    }
  }
  if (!themes_.contains(resolvedTheme)) {
    resolvedTheme = fallbackTheme;
  }

  QString resolvedMode = normalizeModeName(storedModeRaw);
  if (!modeNames().contains(resolvedMode)) {
    resolvedMode = QStringLiteral("light");
  }

  currentTheme_ = resolvedTheme;
  currentMode_ = resolvedMode;
  loadTheme(currentTheme_, currentMode_);
}

void ThemeProvider::initializeThemes() {
  if (!themeNames_.isEmpty()) {
    return;
  }

  registerTheme(QStringLiteral("Diffy"), builtInDarkThemeColors(), builtInLightThemeColors());

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
  QString resolvedTheme = name.trimmed();

  if (!themes_.contains(resolvedTheme)) {
    for (const QString& candidate : themeNames_) {
      if (candidate.compare(resolvedTheme, Qt::CaseInsensitive) == 0) {
        resolvedTheme = candidate;
        break;
      }
    }
  }

  if (!themes_.contains(resolvedTheme)) {
    resolvedTheme = themes_.contains(QStringLiteral("Diffy")) ? QStringLiteral("Diffy") : themeNames_.value(0);
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
    const QString fallback =
        themes_.contains(QStringLiteral("Diffy")) ? QStringLiteral("Diffy") : themeNames_.value(0);
    if (fallback.isEmpty()) {
      return;
    }
    const ThemeVariants fallbackVariants = themes_.value(fallback);
    colors_ = mode == "light" ? fallbackVariants.light : fallbackVariants.dark;
    return;
  }

  const ThemeVariants variants = themes_.value(name);
  colors_ = mode == "light" ? variants.light : variants.dark;
}

}  // namespace diffy
