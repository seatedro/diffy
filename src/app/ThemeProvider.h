#pragma once

#include <QColor>
#include <QObject>

namespace diffy {

class ThemeProvider : public QObject {
  Q_OBJECT

  Q_PROPERTY(QString sans MEMBER sans_ CONSTANT)
  Q_PROPERTY(QString mono MEMBER mono_ CONSTANT)

  Q_PROPERTY(QColor appBg MEMBER appBg_ CONSTANT)
  Q_PROPERTY(QColor canvas MEMBER canvas_ CONSTANT)
  Q_PROPERTY(QColor panel MEMBER panel_ CONSTANT)
  Q_PROPERTY(QColor panelStrong MEMBER panelStrong_ CONSTANT)
  Q_PROPERTY(QColor panelTint MEMBER panelTint_ CONSTANT)
  Q_PROPERTY(QColor toolbarBg MEMBER toolbarBg_ CONSTANT)

  Q_PROPERTY(QColor borderSoft MEMBER borderSoft_ CONSTANT)
  Q_PROPERTY(QColor borderStrong MEMBER borderStrong_ CONSTANT)
  Q_PROPERTY(QColor divider MEMBER divider_ CONSTANT)

  Q_PROPERTY(QColor textStrong MEMBER textStrong_ CONSTANT)
  Q_PROPERTY(QColor textBase MEMBER textBase_ CONSTANT)
  Q_PROPERTY(QColor textMuted MEMBER textMuted_ CONSTANT)
  Q_PROPERTY(QColor textFaint MEMBER textFaint_ CONSTANT)

  Q_PROPERTY(QColor accent MEMBER accent_ CONSTANT)
  Q_PROPERTY(QColor accentStrong MEMBER accentStrong_ CONSTANT)
  Q_PROPERTY(QColor accentSoft MEMBER accentSoft_ CONSTANT)

  Q_PROPERTY(QColor successBg MEMBER successBg_ CONSTANT)
  Q_PROPERTY(QColor successBorder MEMBER successBorder_ CONSTANT)
  Q_PROPERTY(QColor successText MEMBER successText_ CONSTANT)

  Q_PROPERTY(QColor dangerBg MEMBER dangerBg_ CONSTANT)
  Q_PROPERTY(QColor dangerBorder MEMBER dangerBorder_ CONSTANT)
  Q_PROPERTY(QColor dangerText MEMBER dangerText_ CONSTANT)

  Q_PROPERTY(QColor warningBg MEMBER warningBg_ CONSTANT)
  Q_PROPERTY(QColor warningBorder MEMBER warningBorder_ CONSTANT)
  Q_PROPERTY(QColor warningText MEMBER warningText_ CONSTANT)

  Q_PROPERTY(QColor selectionBg MEMBER selectionBg_ CONSTANT)
  Q_PROPERTY(QColor selectionBorder MEMBER selectionBorder_ CONSTANT)

  Q_PROPERTY(QColor lineContext MEMBER lineContext_ CONSTANT)
  Q_PROPERTY(QColor lineContextAlt MEMBER lineContextAlt_ CONSTANT)
  Q_PROPERTY(QColor lineAdd MEMBER lineAdd_ CONSTANT)
  Q_PROPERTY(QColor lineAddAccent MEMBER lineAddAccent_ CONSTANT)
  Q_PROPERTY(QColor lineDel MEMBER lineDel_ CONSTANT)
  Q_PROPERTY(QColor lineDelAccent MEMBER lineDelAccent_ CONSTANT)

  // Spacing scale (4px grid)
  Q_PROPERTY(int sp1 MEMBER sp1_ CONSTANT)   // 4
  Q_PROPERTY(int sp2 MEMBER sp2_ CONSTANT)   // 8
  Q_PROPERTY(int sp3 MEMBER sp3_ CONSTANT)   // 12
  Q_PROPERTY(int sp4 MEMBER sp4_ CONSTANT)   // 16
  Q_PROPERTY(int sp6 MEMBER sp6_ CONSTANT)   // 24
  Q_PROPERTY(int sp8 MEMBER sp8_ CONSTANT)   // 32
  Q_PROPERTY(int sp12 MEMBER sp12_ CONSTANT) // 48

  // Typography scale
  Q_PROPERTY(int fontCaption MEMBER fontCaption_ CONSTANT)
  Q_PROPERTY(int fontSmall MEMBER fontSmall_ CONSTANT)
  Q_PROPERTY(int fontBody MEMBER fontBody_ CONSTANT)
  Q_PROPERTY(int fontSubtitle MEMBER fontSubtitle_ CONSTANT)
  Q_PROPERTY(int fontTitle MEMBER fontTitle_ CONSTANT)
  Q_PROPERTY(int fontHeading MEMBER fontHeading_ CONSTANT)

  // Border-radius scale
  Q_PROPERTY(int radiusSm MEMBER radiusSm_ CONSTANT)
  Q_PROPERTY(int radiusMd MEMBER radiusMd_ CONSTANT)
  Q_PROPERTY(int radiusLg MEMBER radiusLg_ CONSTANT)
  Q_PROPERTY(int radiusXl MEMBER radiusXl_ CONSTANT)

  // Elevation shadow colors
  Q_PROPERTY(QColor shadowSm MEMBER shadowSm_ CONSTANT)
  Q_PROPERTY(QColor shadowMd MEMBER shadowMd_ CONSTANT)
  Q_PROPERTY(QColor shadowLg MEMBER shadowLg_ CONSTANT)

 public:
  explicit ThemeProvider(QObject* parent = nullptr) : QObject(parent) {}

 private:
  QString sans_ = "IBM Plex Sans";
  QString mono_ = "JetBrains Mono";

  QColor appBg_{"#1d2021"};
  QColor canvas_{"#282828"};
  QColor panel_{"#32302f"};
  QColor panelStrong_{"#3c3836"};
  QColor panelTint_{"#504945"};
  QColor toolbarBg_{"#282828"};

  QColor borderSoft_{"#504945"};
  QColor borderStrong_{"#665c54"};
  QColor divider_{"#504945"};

  QColor textStrong_{"#fbf1c7"};
  QColor textBase_{"#ebdbb2"};
  QColor textMuted_{"#d5c4a1"};
  QColor textFaint_{"#a89984"};

  QColor accent_{"#83a598"};
  QColor accentStrong_{"#83a598"};
  QColor accentSoft_{"#3b4b4f"};

  QColor successBg_{"#32361a"};
  QColor successBorder_{"#4a5a1c"};
  QColor successText_{"#b8bb26"};

  QColor dangerBg_{"#3c1f1e"};
  QColor dangerBorder_{"#7c3a31"};
  QColor dangerText_{"#fb4934"};

  QColor warningBg_{"#4a3b16"};
  QColor warningBorder_{"#7c6f27"};
  QColor warningText_{"#fabd2f"};

  QColor selectionBg_{"#3c3836"};
  QColor selectionBorder_{"#83a598"};

  QColor lineContext_{"#282828"};
  QColor lineContextAlt_{"#232323"};
  QColor lineAdd_{"#2a3a1e"};
  QColor lineAddAccent_{"#324420"};
  QColor lineDel_{"#3d2020"};
  QColor lineDelAccent_{"#4c2828"};

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

  QColor shadowSm_{"#1a000000"};
  QColor shadowMd_{"#33000000"};
  QColor shadowLg_{"#4d000000"};
};

}  // namespace diffy
