#pragma once

#include <QObject>

class AppSmokeTest : public QObject {
  Q_OBJECT

 private slots:
  void initTestCase();
  void launchesUnifiedAndPrintsSurfaceState();
  void launchesSplitSecondFileAndPrintsSurfaceState();
  void scrollsUnifiedViewportWithoutShrinkingSurface();
  void wheelScrollsSplitViewportDespiteHorizontalTrackpadNoise();
  void switchesFromSplitToUnifiedWhileScrolled();
  void warmSplitReverseWheelDoesNotUploadMoreTextures();
  void opensInAppRepositoryPickerWithoutWarnings();
};
