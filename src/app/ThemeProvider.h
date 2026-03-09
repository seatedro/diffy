#pragma once

#include <QColor>
#include <QObject>
#include <QSettings>
#include <QStringList>

namespace diffy {

struct ThemeColors {
  QColor appBg;
  QColor canvas;
  QColor panel;
  QColor panelStrong;
  QColor panelTint;
  QColor toolbarBg;

  QColor borderSoft;
  QColor borderStrong;
  QColor divider;

  QColor textStrong;
  QColor textBase;
  QColor textMuted;
  QColor textFaint;

  QColor accent;
  QColor accentStrong;
  QColor accentSoft;

  QColor successBg;
  QColor successBorder;
  QColor successText;

  QColor dangerBg;
  QColor dangerBorder;
  QColor dangerText;

  QColor warningBg;
  QColor warningBorder;
  QColor warningText;

  QColor selectionBg;
  QColor selectionBorder;

  QColor lineContext;
  QColor lineContextAlt;
  QColor lineAdd;
  QColor lineAddAccent;
  QColor lineDel;
  QColor lineDelAccent;

  QColor shadowSm;
  QColor shadowMd;
  QColor shadowLg;
};

class ThemeProvider : public QObject {
  Q_OBJECT

  Q_PROPERTY(QString sans MEMBER sans_ CONSTANT)
  Q_PROPERTY(QString mono MEMBER mono_ CONSTANT)

  Q_PROPERTY(QString currentTheme READ currentTheme NOTIFY themeChanged)
  Q_PROPERTY(QStringList availableThemes READ availableThemes CONSTANT)

  Q_PROPERTY(QColor appBg READ appBg NOTIFY themeChanged)
  Q_PROPERTY(QColor canvas READ canvas NOTIFY themeChanged)
  Q_PROPERTY(QColor panel READ panel NOTIFY themeChanged)
  Q_PROPERTY(QColor panelStrong READ panelStrong NOTIFY themeChanged)
  Q_PROPERTY(QColor panelTint READ panelTint NOTIFY themeChanged)
  Q_PROPERTY(QColor toolbarBg READ toolbarBg NOTIFY themeChanged)

  Q_PROPERTY(QColor borderSoft READ borderSoft NOTIFY themeChanged)
  Q_PROPERTY(QColor borderStrong READ borderStrong NOTIFY themeChanged)
  Q_PROPERTY(QColor divider READ divider NOTIFY themeChanged)

  Q_PROPERTY(QColor textStrong READ textStrong NOTIFY themeChanged)
  Q_PROPERTY(QColor textBase READ textBase NOTIFY themeChanged)
  Q_PROPERTY(QColor textMuted READ textMuted NOTIFY themeChanged)
  Q_PROPERTY(QColor textFaint READ textFaint NOTIFY themeChanged)

  Q_PROPERTY(QColor accent READ accent NOTIFY themeChanged)
  Q_PROPERTY(QColor accentStrong READ accentStrong NOTIFY themeChanged)
  Q_PROPERTY(QColor accentSoft READ accentSoft NOTIFY themeChanged)

  Q_PROPERTY(QColor successBg READ successBg NOTIFY themeChanged)
  Q_PROPERTY(QColor successBorder READ successBorder NOTIFY themeChanged)
  Q_PROPERTY(QColor successText READ successText NOTIFY themeChanged)

  Q_PROPERTY(QColor dangerBg READ dangerBg NOTIFY themeChanged)
  Q_PROPERTY(QColor dangerBorder READ dangerBorder NOTIFY themeChanged)
  Q_PROPERTY(QColor dangerText READ dangerText NOTIFY themeChanged)

  Q_PROPERTY(QColor warningBg READ warningBg NOTIFY themeChanged)
  Q_PROPERTY(QColor warningBorder READ warningBorder NOTIFY themeChanged)
  Q_PROPERTY(QColor warningText READ warningText NOTIFY themeChanged)

  Q_PROPERTY(QColor selectionBg READ selectionBg NOTIFY themeChanged)
  Q_PROPERTY(QColor selectionBorder READ selectionBorder NOTIFY themeChanged)

  Q_PROPERTY(QColor lineContext READ lineContext NOTIFY themeChanged)
  Q_PROPERTY(QColor lineContextAlt READ lineContextAlt NOTIFY themeChanged)
  Q_PROPERTY(QColor lineAdd READ lineAdd NOTIFY themeChanged)
  Q_PROPERTY(QColor lineAddAccent READ lineAddAccent NOTIFY themeChanged)
  Q_PROPERTY(QColor lineDel READ lineDel NOTIFY themeChanged)
  Q_PROPERTY(QColor lineDelAccent READ lineDelAccent NOTIFY themeChanged)

  Q_PROPERTY(QColor shadowSm READ shadowSm NOTIFY themeChanged)
  Q_PROPERTY(QColor shadowMd READ shadowMd NOTIFY themeChanged)
  Q_PROPERTY(QColor shadowLg READ shadowLg NOTIFY themeChanged)

  Q_PROPERTY(int sp1 MEMBER sp1_ CONSTANT)
  Q_PROPERTY(int sp2 MEMBER sp2_ CONSTANT)
  Q_PROPERTY(int sp3 MEMBER sp3_ CONSTANT)
  Q_PROPERTY(int sp4 MEMBER sp4_ CONSTANT)
  Q_PROPERTY(int sp6 MEMBER sp6_ CONSTANT)
  Q_PROPERTY(int sp8 MEMBER sp8_ CONSTANT)
  Q_PROPERTY(int sp12 MEMBER sp12_ CONSTANT)

  Q_PROPERTY(int fontCaption MEMBER fontCaption_ CONSTANT)
  Q_PROPERTY(int fontSmall MEMBER fontSmall_ CONSTANT)
  Q_PROPERTY(int fontBody MEMBER fontBody_ CONSTANT)
  Q_PROPERTY(int fontSubtitle MEMBER fontSubtitle_ CONSTANT)
  Q_PROPERTY(int fontTitle MEMBER fontTitle_ CONSTANT)
  Q_PROPERTY(int fontHeading MEMBER fontHeading_ CONSTANT)

  Q_PROPERTY(int radiusSm MEMBER radiusSm_ CONSTANT)
  Q_PROPERTY(int radiusMd MEMBER radiusMd_ CONSTANT)
  Q_PROPERTY(int radiusLg MEMBER radiusLg_ CONSTANT)
  Q_PROPERTY(int radiusXl MEMBER radiusXl_ CONSTANT)

 public:
  explicit ThemeProvider(QObject* parent = nullptr);

  QString currentTheme() const;
  QStringList availableThemes() const;
  Q_INVOKABLE void setTheme(const QString& name);

  QColor appBg() const { return colors_.appBg; }
  QColor canvas() const { return colors_.canvas; }
  QColor panel() const { return colors_.panel; }
  QColor panelStrong() const { return colors_.panelStrong; }
  QColor panelTint() const { return colors_.panelTint; }
  QColor toolbarBg() const { return colors_.toolbarBg; }

  QColor borderSoft() const { return colors_.borderSoft; }
  QColor borderStrong() const { return colors_.borderStrong; }
  QColor divider() const { return colors_.divider; }

  QColor textStrong() const { return colors_.textStrong; }
  QColor textBase() const { return colors_.textBase; }
  QColor textMuted() const { return colors_.textMuted; }
  QColor textFaint() const { return colors_.textFaint; }

  QColor accent() const { return colors_.accent; }
  QColor accentStrong() const { return colors_.accentStrong; }
  QColor accentSoft() const { return colors_.accentSoft; }

  QColor successBg() const { return colors_.successBg; }
  QColor successBorder() const { return colors_.successBorder; }
  QColor successText() const { return colors_.successText; }

  QColor dangerBg() const { return colors_.dangerBg; }
  QColor dangerBorder() const { return colors_.dangerBorder; }
  QColor dangerText() const { return colors_.dangerText; }

  QColor warningBg() const { return colors_.warningBg; }
  QColor warningBorder() const { return colors_.warningBorder; }
  QColor warningText() const { return colors_.warningText; }

  QColor selectionBg() const { return colors_.selectionBg; }
  QColor selectionBorder() const { return colors_.selectionBorder; }

  QColor lineContext() const { return colors_.lineContext; }
  QColor lineContextAlt() const { return colors_.lineContextAlt; }
  QColor lineAdd() const { return colors_.lineAdd; }
  QColor lineAddAccent() const { return colors_.lineAddAccent; }
  QColor lineDel() const { return colors_.lineDel; }
  QColor lineDelAccent() const { return colors_.lineDelAccent; }

  QColor shadowSm() const { return colors_.shadowSm; }
  QColor shadowMd() const { return colors_.shadowMd; }
  QColor shadowLg() const { return colors_.shadowLg; }

 signals:
  void themeChanged();

 private:
  void loadTheme(const QString& name);
  static ThemeColors gruvboxDark();
  static ThemeColors gruvboxLight();
  static ThemeColors kanagawaDark();
  static ThemeColors kanagawaLight();
  static ThemeColors rosePineDark();
  static ThemeColors rosePineLight();
  static ThemeColors catppuccinDark();
  static ThemeColors catppuccinLight();

  QString currentTheme_;
  ThemeColors colors_;
  QSettings settings_;

  QString sans_ = "IBM Plex Sans";
  QString mono_ = "JetBrains Mono";

  int sp1_ = 4;
  int sp2_ = 8;
  int sp3_ = 12;
  int sp4_ = 16;
  int sp6_ = 24;
  int sp8_ = 32;
  int sp12_ = 48;

  int fontCaption_ = 9;
  int fontSmall_ = 10;
  int fontBody_ = 12;
  int fontSubtitle_ = 14;
  int fontTitle_ = 18;
  int fontHeading_ = 24;

  int radiusSm_ = 4;
  int radiusMd_ = 6;
  int radiusLg_ = 8;
  int radiusXl_ = 12;
};

}  // namespace diffy
