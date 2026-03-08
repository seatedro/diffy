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
  void switchesFilesAndKeepsTimingMetricsAvailable();
  void longChangedLineSupportsSplitHorizontalScrollAndTimingMetrics();
  void splitDeletedFileKeepsSpacerBackgroundInBlankPane();
  void opensInAppRepositoryPickerWithoutWarnings();
};
