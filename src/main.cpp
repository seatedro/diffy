#include <cstdio>
#include <atomic>
#include <algorithm>

#include <QDir>
#include <QFileInfo>
#include <QFileSystemWatcher>
#include <QGuiApplication>
#include <QStandardPaths>
#include <QDebug>
#include <QImage>
#include <QLibraryInfo>
#include <QApplication>
#include <QTimer>
#include <QQmlComponent>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQmlError>
#include <QUrl>
#include <QQuickWindow>
#include <qqml.h>

#include "app/DiffController.h"
#include "app/ThemeProvider.h"
#include "core/support/Log.h"
#include "app/quick/DiffSurfaceItem.h"

namespace {

std::atomic<bool> g_runtimeWarningSeen = false;
QtMessageHandler g_previousMessageHandler = nullptr;

void diffyMessageHandler(QtMsgType type, const QMessageLogContext& context, const QString& message) {
  if (type == QtWarningMsg) {
    g_runtimeWarningSeen.store(true);
  }
  const QByteArray localMessage = message.toLocal8Bit();
  std::fprintf(stderr, "%s\n", localMessage.constData());
  std::fflush(stderr);
  if (g_previousMessageHandler != nullptr) {
    g_previousMessageHandler(type, context, message);
  }
}

bool envFlagEnabled(const char* name) {
  const QByteArray value = qgetenv(name).trimmed().toLower();
  return !value.isEmpty() && value != "0" && value != "false" && value != "no";
}

QString envString(const char* name) {
  return QString::fromLocal8Bit(qgetenv(name));
}

QString discoverQmlSourceRoot() {
  QDir dir(QCoreApplication::applicationDirPath());
  for (int depth = 0; depth < 6; ++depth) {
    if (QFileInfo::exists(dir.filePath("qml/Main.qml"))) {
      return dir.absolutePath();
    }
    if (!dir.cdUp()) {
      break;
    }
  }
  return {};
}

QUrl mainQmlUrl(QQmlApplicationEngine* engine) {
  const QUrl resourceUrl(QStringLiteral("qrc:/Diffy/qml/Main.qml"));
  if (!envFlagEnabled("DIFFY_QML_SOURCE")) {
    return resourceUrl;
  }

  const QString sourceRoot = discoverQmlSourceRoot();
  if (sourceRoot.isEmpty()) {
    std::fprintf(stderr,
                 "DIFFY_QML_SOURCE=1 was set but qml/Main.qml could not be found relative to %s\n",
                 qPrintable(QCoreApplication::applicationDirPath()));
    std::fflush(stderr);
    return resourceUrl;
  }

  const QString qmlDir = QDir(sourceRoot).filePath("qml");
  engine->addImportPath(qmlDir);
  return QUrl::fromLocalFile(QDir(qmlDir).filePath("Main.qml"));
}

void printAutomationState(QObject* root, const diffy::DiffController& controller, const diffy::ThemeProvider& themeProvider) {
  QObject* surface = root != nullptr ? root->findChild<QObject*>("diffSurface") : nullptr;
  const double surfaceHeight = surface != nullptr ? surface->property("contentHeight").toDouble() : -1.0;
  const double surfaceWidth = surface != nullptr ? surface->property("contentWidth").toDouble() : -1.0;
  const double surfaceItemWidth = surface != nullptr ? surface->property("width").toDouble() : -1.0;
  const double surfaceItemHeight = surface != nullptr ? surface->property("height").toDouble() : -1.0;
  const double viewportY = surface != nullptr ? surface->property("viewportY").toDouble() : -1.0;
  const double leftViewportX = surface != nullptr ? surface->property("leftViewportX").toDouble() : -1.0;
  const double rightViewportX = surface != nullptr ? surface->property("rightViewportX").toDouble() : -1.0;
  const int paintCount = surface != nullptr ? surface->property("paintCount").toInt() : -1;
  const int displayRowCount = surface != nullptr ? surface->property("displayRowCount").toInt() : -1;
  const int tileCacheHits = surface != nullptr ? surface->property("tileCacheHits").toInt() : -1;
  const int tileCacheMisses = surface != nullptr ? surface->property("tileCacheMisses").toInt() : -1;
  const int textureUploads = surface != nullptr ? surface->property("textureUploadCount").toInt() : -1;
  const int residentTiles = surface != nullptr ? surface->property("residentTileCount").toInt() : -1;
  const int pendingTileJobs = surface != nullptr ? surface->property("pendingTileJobCount").toInt() : -1;
  const double lastPaintTimeMs = surface != nullptr ? surface->property("lastPaintTimeMs").toDouble() : -1.0;
  const double lastRasterTimeMs = surface != nullptr ? surface->property("lastRasterTimeMs").toDouble() : -1.0;
  const double lastTextureUploadTimeMs =
      surface != nullptr ? surface->property("lastTextureUploadTimeMs").toDouble() : -1.0;
  const double lastRowsRebuildTimeMs =
      surface != nullptr ? surface->property("lastRowsRebuildTimeMs").toDouble() : -1.0;
  const double lastDisplayRowsRebuildTimeMs =
      surface != nullptr ? surface->property("lastDisplayRowsRebuildTimeMs").toDouble() : -1.0;
  const double lastMetricsRecalcTimeMs =
      surface != nullptr ? surface->property("lastMetricsRecalcTimeMs").toDouble() : -1.0;
  const int pickerVisible = controller.repositoryPickerVisible() ? 1 : 0;
  const QString errorText = controller.errorMessage().isEmpty() ? "none" : controller.errorMessage().simplified();
  const QString layout = controller.layoutMode().isEmpty() ? "none" : controller.layoutMode();
  const QString currentView = controller.currentView();
  const QString themeName = themeProvider.currentTheme().isEmpty() ? QStringLiteral("none") : themeProvider.currentTheme();
  const QString themeMode = themeProvider.currentMode().isEmpty() ? QStringLiteral("none") : themeProvider.currentMode();
  const QString themeBg = themeProvider.appBg().isValid() ? themeProvider.appBg().name(QColor::HexArgb) : QStringLiteral("invalid");
  const int themeCount = themeProvider.availableThemes().size();
  const QString selectedPath = controller.selectedFile().value("path").toString();
  const QByteArray encodedSelectedPath =
      QUrl::toPercentEncoding(selectedPath.isEmpty() ? QStringLiteral("none") : selectedPath, "/._-");

  std::fprintf(stdout,
               "DIFFY_STATE current_view=%s files=%d rows=%d selected=%d layout=%s surface_height=%.1f surface_width=%.1f item_width=%.1f item_height=%.1f viewport_y=%.1f left_viewport_x=%.1f right_viewport_x=%.1f selected_path=%s display_rows=%d paint_count=%d tile_cache_hits=%d tile_cache_misses=%d texture_uploads=%d resident_tiles=%d pending_tile_jobs=%d last_paint_ms=%.3f last_raster_ms=%.3f last_upload_ms=%.3f last_rows_rebuild_ms=%.3f last_display_rows_rebuild_ms=%.3f last_metrics_ms=%.3f picker_visible=%d error=%s theme=%s mode=%s theme_bg=%s theme_count=%d\n",
               qPrintable(currentView), static_cast<int>(controller.files().size()), controller.selectedFileRowCount(),
               controller.selectedFileIndex(), qPrintable(layout), surfaceHeight, surfaceWidth,
               surfaceItemWidth, surfaceItemHeight, viewportY, leftViewportX, rightViewportX,
               encodedSelectedPath.constData(), displayRowCount, paintCount, tileCacheHits, tileCacheMisses,
               textureUploads, residentTiles, pendingTileJobs, lastPaintTimeMs, lastRasterTimeMs,
               lastTextureUploadTimeMs, lastRowsRebuildTimeMs, lastDisplayRowsRebuildTimeMs, lastMetricsRecalcTimeMs,
               pickerVisible, qPrintable(errorText), qPrintable(themeName), qPrintable(themeMode), qPrintable(themeBg),
               themeCount);
  std::fflush(stdout);
}

bool applyStartupAutomation(diffy::DiffController* controller, QString* error) {
  const QString repo = envString("DIFFY_START_REPO");
  if (!repo.isEmpty() && !controller->openRepository(repo)) {
    if (error != nullptr) {
      *error = controller->errorMessage();
    }
    return false;
  }

  const QString leftRef = envString("DIFFY_START_LEFT");
  if (!leftRef.isEmpty()) {
    controller->setLeftRef(leftRef);
  }

  const QString rightRef = envString("DIFFY_START_RIGHT");
  if (!rightRef.isEmpty()) {
    controller->setRightRef(rightRef);
  }

  const QString compareMode = envString("DIFFY_START_COMPARE_MODE");
  if (!compareMode.isEmpty()) {
    controller->setCompareMode(compareMode);
  }

  const QString layoutMode = envString("DIFFY_START_LAYOUT");
  if (!layoutMode.isEmpty()) {
    controller->setLayoutMode(layoutMode);
  }

  const QString renderer = envString("DIFFY_START_RENDERER");
  if (!renderer.isEmpty()) {
    controller->setRenderer(renderer);
  }

  if (envFlagEnabled("DIFFY_OPEN_REPO_PICKER")) {
    controller->openRepositoryPicker();
  }

  if (envFlagEnabled("DIFFY_START_COMPARE")) {
    controller->compare();
    while (controller->comparing()) {
      QCoreApplication::processEvents(QEventLoop::AllEvents, 100);
    }
  }

  const auto selectFileByPath = [controller](const QString& selectedFilePath) -> bool {
    const QVariantList files = controller->files();
    for (int index = 0; index < files.size(); ++index) {
      const QString path = files.at(index).toMap().value("path").toString();
      if (path == selectedFilePath || path.endsWith(selectedFilePath)) {
        controller->selectFile(index);
        return true;
      }
    }
    return false;
  };

  bool ok = false;
  const int selectedFileIndex = envString("DIFFY_START_FILE_INDEX").toInt(&ok);
  if (ok) {
    controller->selectFile(selectedFileIndex);
  }

  const QString selectedFilePath = envString("DIFFY_START_FILE_PATH");
  if (!selectedFilePath.isEmpty()) {
    selectFileByPath(selectedFilePath);
  }

  if (envFlagEnabled("DIFFY_PRINT_FILE_LIST")) {
    const QVariantList files = controller->files();
    for (int index = 0; index < files.size(); ++index) {
      const QString path = files.at(index).toMap().value("path").toString();
      std::fprintf(stdout, "DIFFY_FILE index=%d path=%s\n", index, qPrintable(path));
    }
  }

  if (envFlagEnabled("DIFFY_REQUIRE_RESULTS")) {
    if (controller->files().isEmpty()) {
      if (error != nullptr) {
        *error = controller->errorMessage().isEmpty() ? "Startup compare produced no files"
                                                      : controller->errorMessage();
      }
      return false;
    }
    if (controller->selectedFileRowCount() == 0) {
      if (error != nullptr) {
        *error = controller->errorMessage().isEmpty() ? "Startup compare produced no visible rows"
                                                      : controller->errorMessage();
      }
      return false;
    }
  }

  return true;
}

}  // namespace

int main(int argc, char* argv[]) {
  QApplication app(argc, argv);
  QApplication::setOrganizationName("diffy");
  QApplication::setApplicationName("diffy");
  app.addLibraryPath(QLibraryInfo::path(QLibraryInfo::PluginsPath));

  const QString logDir = QStandardPaths::writableLocation(QStandardPaths::AppDataLocation);
  QDir().mkpath(logDir);
  diffy::log::init((logDir + "/diffy.log").toStdString());
  if (envFlagEnabled("DIFFY_LOG_DEBUG")) {
    diffy::log::setLevel(spdlog::level::debug);
  }
  diffy::log::info("app", "diffy starting");

  const bool fatalRuntimeWarnings = envFlagEnabled("DIFFY_FATAL_RUNTIME_WARNINGS");
  g_runtimeWarningSeen.store(false);
  g_previousMessageHandler = qInstallMessageHandler(diffyMessageHandler);

  qmlRegisterType<diffy::DiffSurfaceItem>("Diffy.Native", 1, 0, "DiffSurface");

  QQmlApplicationEngine engine;
  engine.setOutputWarningsToStandardError(true);
  engine.addImportPath(QLibraryInfo::path(QLibraryInfo::QmlImportsPath));

  const auto addQtPrefixPaths = [&](const QString& prefix) {
    const QString qmlPath = QDir(prefix).filePath("lib/qt-6/qml");
    const QString pluginPath = QDir(prefix).filePath("lib/qt-6/plugins");
    if (QFileInfo::exists(qmlPath)) {
      engine.addImportPath(qmlPath);
    }
    if (QFileInfo::exists(pluginPath)) {
      app.addLibraryPath(pluginPath);
    }
  };

  const QString additionalPrefixes =
      QString::fromLocal8Bit(qgetenv("QT_ADDITIONAL_PACKAGES_PREFIX_PATH"));
  for (const QString& prefix : additionalPrefixes.split(':', Qt::SkipEmptyParts)) {
    addQtPrefixPaths(prefix);
  }

  const QString extraQmlImports = QString::fromLocal8Bit(qgetenv("QML2_IMPORT_PATH"));
  for (const QString& importPath : extraQmlImports.split(':', Qt::SkipEmptyParts)) {
    if (QFileInfo::exists(importPath)) {
      engine.addImportPath(importPath);
    }
  }

  QObject::connect(&engine, &QQmlApplicationEngine::warnings,
                   [](const QList<QQmlError>& warnings) {
                     for (const QQmlError& warning : warnings) {
                       qWarning().noquote() << warning.toString();
                     }
                   });

  diffy::ThemeProvider themeProvider;
  diffy::DiffController controller;

  QString automationError;
  if (!applyStartupAutomation(&controller, &automationError)) {
    std::fprintf(stderr, "Startup automation failed: %s\n", qPrintable(automationError));
    return 1;
  }

  engine.rootContext()->setContextProperty("theme", &themeProvider);
  engine.rootContext()->setContextProperty("diffController", &controller);

  const QUrl mainUrl = mainQmlUrl(&engine);
  QQmlComponent component(&engine, mainUrl);
  if (component.isError()) {
    const QList<QQmlError> errors = component.errors();
    std::fprintf(stderr, "Failed to load QML component (%d errors)\n", static_cast<int>(errors.size()));
    for (const QQmlError& error : errors) {
      std::fprintf(stderr, "%s\n", qPrintable(error.toString()));
      qWarning().noquote() << error.toString();
    }
    return 1;
  }

  QObject* root = component.create(engine.rootContext());
  if (root == nullptr) {
    const QList<QQmlError> errors = component.errors();
    std::fprintf(stderr, "Failed to create root object (%d errors)\n", static_cast<int>(errors.size()));
    for (const QQmlError& error : errors) {
      std::fprintf(stderr, "%s\n", qPrintable(error.toString()));
      qWarning().noquote() << error.toString();
    }
    return 1;
  }

  QObject::connect(&app, &QCoreApplication::aboutToQuit, root, &QObject::deleteLater);

  if (envFlagEnabled("DIFFY_DEBUG")) {
    QObject::connect(&app, &QCoreApplication::aboutToQuit, [root]() {
      if (auto* surface = root ? root->findChild<diffy::DiffSurfaceItem*>("diffSurface") : nullptr) {
        surface->dumpPerfReport();
      }
    });
  }

  if (envFlagEnabled("DIFFY_PRINT_STATE")) {
    bool delayOk = false;
    bool repeatOk = false;
    bool countOk = false;
    const int printStateDelayMs = envString("DIFFY_PRINT_STATE_DELAY_MS").toInt(&delayOk);
    const int printStateRepeatMs = envString("DIFFY_PRINT_STATE_REPEAT_MS").toInt(&repeatOk);
    const int printStateCount = envString("DIFFY_PRINT_STATE_COUNT").toInt(&countOk);
    const int firstDelayMs = delayOk ? printStateDelayMs : 100;
    const int repeatDelayMs = repeatOk ? printStateRepeatMs : 0;
    const int totalPrints = std::max(1, countOk ? printStateCount : 1);

    for (int printIndex = 0; printIndex < totalPrints; ++printIndex) {
      QTimer::singleShot(firstDelayMs + repeatDelayMs * printIndex, &app, [&controller, &themeProvider, root]() {
        printAutomationState(root, controller, themeProvider);
      });
    }
  }

  const QString resetPerfStatsAfter = envString("DIFFY_RESET_SURFACE_STATS_AFTER_MS");
  if (!resetPerfStatsAfter.isEmpty()) {
    bool resetOk = false;
    const int resetDelayMs = resetPerfStatsAfter.toInt(&resetOk);
    QTimer::singleShot(resetOk ? resetDelayMs : 160, &app, [root]() {
      if (QObject* surface = root != nullptr ? root->findChild<QObject*>("diffSurface") : nullptr) {
        QMetaObject::invokeMethod(surface, "resetPerfStats");
      }
    });
  }

  const QString startScrollY = envString("DIFFY_START_SCROLL_Y");
  if (!startScrollY.isEmpty()) {
    bool ok = false;
    const double scrollY = startScrollY.toDouble(&ok);
    if (ok && controller.currentView() == "diff") {
      QTimer::singleShot(120, &app, [root, scrollY]() {
        if (QObject* viewport = root != nullptr ? root->findChild<QObject*>("diffViewport") : nullptr) {
          viewport->setProperty("contentY", scrollY);
        }
      });
    }
  }

  const QString switchLayoutTo = envString("DIFFY_SWITCH_LAYOUT_TO");
  if (!switchLayoutTo.isEmpty()) {
    bool switchDelayOk = false;
    const int switchLayoutDelayMs = envString("DIFFY_SWITCH_LAYOUT_AFTER_MS").toInt(&switchDelayOk);
    QTimer::singleShot(switchDelayOk ? switchLayoutDelayMs : 180, &app, [&controller, switchLayoutTo]() {
      controller.setLayoutMode(switchLayoutTo);
    });
  }

  const QString secondSwitchLayoutTo = envString("DIFFY_SWITCH_SECOND_LAYOUT_TO");
  if (!secondSwitchLayoutTo.isEmpty()) {
    bool switchDelayOk = false;
    const int switchLayoutDelayMs = envString("DIFFY_SWITCH_SECOND_LAYOUT_AFTER_MS").toInt(&switchDelayOk);
    QTimer::singleShot(switchDelayOk ? switchLayoutDelayMs : 360, &app, [&controller, secondSwitchLayoutTo]() {
      controller.setLayoutMode(secondSwitchLayoutTo);
    });
  }

  const auto selectFileByPath = [&controller](const QString& selectedFilePath) {
    const QVariantList files = controller.files();
    for (int index = 0; index < files.size(); ++index) {
      const QString path = files.at(index).toMap().value("path").toString();
      if (path == selectedFilePath || path.endsWith(selectedFilePath)) {
        controller.selectFile(index);
        return;
      }
    }
  };

  const QString switchFileToPath = envString("DIFFY_SWITCH_FILE_TO_PATH");
  if (!switchFileToPath.isEmpty()) {
    bool switchDelayOk = false;
    const int switchFileDelayMs = envString("DIFFY_SWITCH_FILE_AFTER_MS").toInt(&switchDelayOk);
    QTimer::singleShot(switchDelayOk ? switchFileDelayMs : 180, &app, [&controller, switchFileToPath, selectFileByPath]() {
      selectFileByPath(switchFileToPath);
    });
  }

  const QString secondSwitchFileToPath = envString("DIFFY_SWITCH_SECOND_FILE_TO_PATH");
  if (!secondSwitchFileToPath.isEmpty()) {
    bool switchDelayOk = false;
    const int switchFileDelayMs = envString("DIFFY_SWITCH_SECOND_FILE_AFTER_MS").toInt(&switchDelayOk);
    QTimer::singleShot(switchDelayOk ? switchFileDelayMs : 360, &app,
                       [&controller, secondSwitchFileToPath, selectFileByPath]() {
      selectFileByPath(secondSwitchFileToPath);
    });
  }

  const auto scheduleWheelEvent = [&app, root](int delayMs, int pixelX, int pixelY, int angleX, int angleY) {
    QTimer::singleShot(delayMs, &app, [root, pixelX, pixelY, angleX, angleY]() {
      auto* surface = root != nullptr ? qobject_cast<QQuickItem*>(root->findChild<QObject*>("diffSurface")) : nullptr;
      if (surface == nullptr) {
        return;
      }

      const QPoint pixelDelta(pixelX, pixelY);
      const QPoint angleDelta(angleX, angleY);
      const QPointF localPos(surface->width() / 2.0, surface->height() / 2.0);
      const QPointF scenePos = surface->mapToScene(localPos);
      QWheelEvent event(localPos, scenePos, pixelDelta, angleDelta, Qt::NoButton, Qt::NoModifier,
                        Qt::ScrollUpdate, false);
      QCoreApplication::sendEvent(surface, &event);
    });
  };

  const QString startWheelPixelX = envString("DIFFY_START_WHEEL_PIXEL_X");
  const QString startWheelPixelY = envString("DIFFY_START_WHEEL_PIXEL_Y");
  const QString startWheelAngleX = envString("DIFFY_START_WHEEL_ANGLE_X");
  const QString startWheelAngleY = envString("DIFFY_START_WHEEL_ANGLE_Y");
  if (!startWheelPixelX.isEmpty() || !startWheelPixelY.isEmpty() || !startWheelAngleX.isEmpty() ||
      !startWheelAngleY.isEmpty()) {
    bool delayOk = false;
    bool pixelXOk = false;
    bool pixelYOk = false;
    bool angleXOk = false;
    bool angleYOk = false;
    const int wheelDelayMs = envString("DIFFY_START_WHEEL_AFTER_MS").toInt(&delayOk);
    const int pixelX = startWheelPixelX.toInt(&pixelXOk);
    const int pixelY = startWheelPixelY.toInt(&pixelYOk);
    const int angleX = startWheelAngleX.toInt(&angleXOk);
    const int angleY = startWheelAngleY.toInt(&angleYOk);
    scheduleWheelEvent(delayOk ? wheelDelayMs : 120, pixelXOk ? pixelX : 0, pixelYOk ? pixelY : 0, angleXOk ? angleX : 0,
                       angleYOk ? angleY : 0);
  }

  const QString secondWheelPixelX = envString("DIFFY_START_SECOND_WHEEL_PIXEL_X");
  const QString secondWheelPixelY = envString("DIFFY_START_SECOND_WHEEL_PIXEL_Y");
  const QString secondWheelAngleX = envString("DIFFY_START_SECOND_WHEEL_ANGLE_X");
  const QString secondWheelAngleY = envString("DIFFY_START_SECOND_WHEEL_ANGLE_Y");
  if (!secondWheelPixelX.isEmpty() || !secondWheelPixelY.isEmpty() || !secondWheelAngleX.isEmpty() ||
      !secondWheelAngleY.isEmpty()) {
    bool delayOk = false;
    bool pixelXOk = false;
    bool pixelYOk = false;
    bool angleXOk = false;
    bool angleYOk = false;
    const int secondWheelDelayMs = envString("DIFFY_START_SECOND_WHEEL_AFTER_MS").toInt(&delayOk);
    const int pixelX = secondWheelPixelX.toInt(&pixelXOk);
    const int pixelY = secondWheelPixelY.toInt(&pixelYOk);
    const int angleX = secondWheelAngleX.toInt(&angleXOk);
    const int angleY = secondWheelAngleY.toInt(&angleYOk);
    scheduleWheelEvent(delayOk ? secondWheelDelayMs : 240, pixelXOk ? pixelX : 0, pixelYOk ? pixelY : 0,
                       angleXOk ? angleX : 0, angleYOk ? angleY : 0);
  }

  const QString capturePath = envString("DIFFY_CAPTURE_PATH");
  if (!capturePath.isEmpty()) {
    bool captureDelayOk = false;
    const int captureDelayMs = envString("DIFFY_CAPTURE_DELAY_MS").toInt(&captureDelayOk);
    QTimer::singleShot(captureDelayOk ? captureDelayMs : 360, &app, [root, capturePath]() {
      if (auto* window = qobject_cast<QQuickWindow*>(root)) {
        const QImage image = window->grabWindow();
        image.save(capturePath);
      }
    });
  }

  bool ok = false;
  const int exitAfterMs = envString("DIFFY_EXIT_AFTER_MS").toInt(&ok);
  if (ok && exitAfterMs >= 0) {
    QTimer::singleShot(exitAfterMs, &app, &QCoreApplication::quit);
  }

  QTimer warningWatcher;
  if (fatalRuntimeWarnings) {
    warningWatcher.setInterval(50);
    QObject::connect(&warningWatcher, &QTimer::timeout, &app, []() {
      if (g_runtimeWarningSeen.load()) {
        QCoreApplication::exit(2);
      }
    });
    warningWatcher.start();
  }

  const int exitCode = app.exec();
  qInstallMessageHandler(g_previousMessageHandler);
  if (exitCode != 0) {
    std::fprintf(stderr, "Application exited with code %d\n", exitCode);
  }
  return exitCode;
}
