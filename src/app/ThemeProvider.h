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
};

}  // namespace diffy
