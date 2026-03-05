#include <cstdio>

#include <QDir>
#include <QFileInfo>
#include <QGuiApplication>
#include <QDebug>
#include <QLibraryInfo>
#include <QQmlComponent>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQmlError>
#include <QUrl>

#include "core/DiffController.h"

int main(int argc, char* argv[]) {
  QGuiApplication app(argc, argv);
  QGuiApplication::setOrganizationName("diffy");
  QGuiApplication::setApplicationName("diffy");
  app.addLibraryPath(QLibraryInfo::path(QLibraryInfo::PluginsPath));

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

  diffy::DiffController controller;
  engine.rootContext()->setContextProperty("diffController", &controller);

  const QUrl mainUrl(QStringLiteral("qrc:/Diffy/qml/Main.qml"));
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

  const int exitCode = app.exec();
  if (exitCode != 0) {
    std::fprintf(stderr, "Application exited with code %d\n", exitCode);
  }
  return exitCode;
}
