#include "app/ThemeProvider.h"

namespace diffy {
namespace {

const QStringList& themeNames() {
  static const QStringList kThemeNames = {
      "gruvbox-dark",
      "gruvbox-light",
      "kanagawa-dark",
      "kanagawa-light",
      "rose-pine-dark",
      "rose-pine-light",
      "catppuccin-dark",
      "catppuccin-light",
  };
  return kThemeNames;
}

QString normalizeThemeName(const QString& name) {
  if (name == "gruvbox") {
    return QStringLiteral("gruvbox-dark");
  }
  if (name == "kanagawa") {
    return QStringLiteral("kanagawa-dark");
  }
  if (name == "rose-pine") {
    return QStringLiteral("rose-pine-dark");
  }
  if (name == "catppuccin") {
    return QStringLiteral("catppuccin-dark");
  }
  return name;
}

}  // namespace

ThemeProvider::ThemeProvider(QObject* parent) : QObject(parent) {
  const QString storedTheme = normalizeThemeName(settings_.value("theme", "gruvbox-dark").toString());
  currentTheme_ = themeNames().contains(storedTheme) ? storedTheme : QStringLiteral("gruvbox-dark");
  loadTheme(currentTheme_);
}

QString ThemeProvider::currentTheme() const {
  return currentTheme_;
}

QStringList ThemeProvider::availableThemes() const {
  return themeNames();
}

void ThemeProvider::setTheme(const QString& name) {
  QString resolvedName = normalizeThemeName(name);
  if (!themeNames().contains(resolvedName)) {
    resolvedName = QStringLiteral("gruvbox-dark");
  }

  if (resolvedName == currentTheme_) {
    return;
  }

  loadTheme(resolvedName);
  currentTheme_ = resolvedName;
  settings_.setValue("theme", resolvedName);
  emit themeChanged();
}

void ThemeProvider::loadTheme(const QString& name) {
  if (name == "gruvbox-light") {
    colors_ = gruvboxLight();
  } else if (name == "kanagawa-dark") {
    colors_ = kanagawaDark();
  } else if (name == "kanagawa-light") {
    colors_ = kanagawaLight();
  } else if (name == "rose-pine-dark") {
    colors_ = rosePineDark();
  } else if (name == "rose-pine-light") {
    colors_ = rosePineLight();
  } else if (name == "catppuccin-dark") {
    colors_ = catppuccinDark();
  } else if (name == "catppuccin-light") {
    colors_ = catppuccinLight();
  } else {
    colors_ = gruvboxDark();
  }
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
      QColor("#1f1f28"), QColor("#2a2a37"), QColor("#363646"), QColor("#3a3a4a"), QColor("#54546D"), QColor("#2a2a37"),
      QColor("#54546D"), QColor("#727169"), QColor("#54546D"),
      QColor("#DCD7BA"), QColor("#C8C093"), QColor("#C8C093"), QColor("#727169"),
      QColor("#7E9CD8"), QColor("#7E9CD8"), QColor("#2D3F5F"),
      QColor("#2B3328"), QColor("#4a6a3a"), QColor("#98BB6C"),
      QColor("#3D2020"), QColor("#7c3a3a"), QColor("#FF5D62"),
      QColor("#3D3020"), QColor("#7c6a3a"), QColor("#E6C384"),
      QColor("#363646"), QColor("#7E9CD8"),
      QColor("#2a2a37"), QColor("#252530"), QColor("#2B3328"), QColor("#334030"), QColor("#3D2020"), QColor("#4a2828"),
      QColor("#1a000000"), QColor("#33000000"), QColor("#4d000000")
  };
}

ThemeColors ThemeProvider::kanagawaLight() {
  return {
      QColor("#f2ecbc"), QColor("#e8e0b8"), QColor("#ddd3af"), QColor("#d0c5a1"), QColor("#b9ad8d"), QColor("#e8e0b8"),
      QColor("#d0c5a1"), QColor("#b9ad8d"), QColor("#d0c5a1"),
      QColor("#1f1f28"), QColor("#2a2a37"), QColor("#545464"), QColor("#716e61"),
      QColor("#4d699b"), QColor("#4d699b"), QColor("#d7deef"),
      QColor("#e4ebd4"), QColor("#8a9b55"), QColor("#6f894e"),
      QColor("#f3d9d7"), QColor("#c4746e"), QColor("#b35b79"),
      QColor("#f2e5cd"), QColor("#dca561"), QColor("#c28a4b"),
      QColor("#ddd3af"), QColor("#4d699b"),
      QColor("#e8e0b8"), QColor("#e3dab2"), QColor("#dde8d0"), QColor("#cfe0bc"), QColor("#edd8d4"), QColor("#e2c6c0"),
      QColor("#0a000000"), QColor("#15000000"), QColor("#22000000")
  };
}

ThemeColors ThemeProvider::rosePineDark() {
  return {
      QColor("#191724"), QColor("#1f1d2e"), QColor("#26233a"), QColor("#2a283e"), QColor("#403d52"), QColor("#1f1d2e"),
      QColor("#403d52"), QColor("#524f67"), QColor("#403d52"),
      QColor("#e0def4"), QColor("#e0def4"), QColor("#908caa"), QColor("#6e6a86"),
      QColor("#c4a7e7"), QColor("#c4a7e7"), QColor("#3a2e58"),
      QColor("#28303a"), QColor("#3e6050"), QColor("#9ccfd8"),
      QColor("#3a2030"), QColor("#6a3050"), QColor("#eb6f92"),
      QColor("#3a3028"), QColor("#6a5838"), QColor("#f6c177"),
      QColor("#26233a"), QColor("#c4a7e7"),
      QColor("#1f1d2e"), QColor("#1a1826"), QColor("#263038"), QColor("#2e3a40"), QColor("#382028"), QColor("#402830"),
      QColor("#1a000000"), QColor("#33000000"), QColor("#4d000000")
  };
}

ThemeColors ThemeProvider::rosePineLight() {
  return {
      QColor("#faf4ed"), QColor("#fffaf3"), QColor("#f2e9e1"), QColor("#e8ded5"), QColor("#cecacd"), QColor("#fffaf3"),
      QColor("#dfdad9"), QColor("#cecacd"), QColor("#dfdad9"),
      QColor("#575279"), QColor("#575279"), QColor("#797593"), QColor("#9893a5"),
      QColor("#907aa9"), QColor("#907aa9"), QColor("#ece3f6"),
      QColor("#e5f0ef"), QColor("#8ab6bd"), QColor("#56949f"),
      QColor("#f2dde4"), QColor("#d7829c"), QColor("#b4637a"),
      QColor("#f7ead8"), QColor("#efb46a"), QColor("#ea9d34"),
      QColor("#f2e9e1"), QColor("#907aa9"),
      QColor("#fffaf3"), QColor("#f4ede8"), QColor("#e7efea"), QColor("#d7e3db"), QColor("#f2e1e5"), QColor("#ead0d8"),
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
      QColor("#dce0e8"), QColor("#1e66f5"),
      QColor("#e6e9ef"), QColor("#dde0e6"), QColor("#e2efdf"), QColor("#d4e7d0"), QColor("#f3dde1"), QColor("#ebccd3"),
      QColor("#0a000000"), QColor("#15000000"), QColor("#22000000")
  };
}

}  // namespace diffy
