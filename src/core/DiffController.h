#pragma once

#include <QObject>
#include <QSettings>
#include <QStringList>
#include <QVariant>

#include "core/CompareSpec.h"
#include "core/GitRepositoryService.h"
#include "core/UnifiedDiffParser.h"
#include "model/DiffRowListModel.h"
#include "renderers/BuiltinGitRenderer.h"
#include "renderers/DifftasticRenderer.h"

namespace diffy {

class DiffController : public QObject {
  Q_OBJECT
  Q_PROPERTY(QString repoPath READ repoPath WRITE setRepoPath NOTIFY repoPathChanged)
  Q_PROPERTY(QStringList refs READ refs NOTIFY refsChanged)
  Q_PROPERTY(QString leftRef READ leftRef WRITE setLeftRef NOTIFY leftRefChanged)
  Q_PROPERTY(QString rightRef READ rightRef WRITE setRightRef NOTIFY rightRefChanged)
  Q_PROPERTY(QString compareMode READ compareMode WRITE setCompareMode NOTIFY compareModeChanged)
  Q_PROPERTY(QString renderer READ renderer WRITE setRenderer NOTIFY rendererChanged)
  Q_PROPERTY(QString layoutMode READ layoutMode WRITE setLayoutMode NOTIFY layoutModeChanged)
  Q_PROPERTY(QVariantList files READ files NOTIFY filesChanged)
  Q_PROPERTY(int selectedFileIndex READ selectedFileIndex WRITE setSelectedFileIndex NOTIFY selectedFileIndexChanged)
  Q_PROPERTY(QVariantMap selectedFile READ selectedFile NOTIFY selectedFileChanged)
  Q_PROPERTY(QObject* selectedFileRowsModel READ selectedFileRowsModel CONSTANT)
  Q_PROPERTY(int selectedFileRowCount READ selectedFileRowCount NOTIFY selectedFileRowsChanged)
  Q_PROPERTY(QString errorMessage READ errorMessage NOTIFY errorMessageChanged)
  Q_PROPERTY(bool hasDifftastic READ hasDifftastic NOTIFY hasDifftasticChanged)

 public:
  explicit DiffController(QObject* parent = nullptr);

  QString repoPath() const;
  void setRepoPath(const QString& path);

  QStringList refs() const;

  QString leftRef() const;
  void setLeftRef(const QString& value);

  QString rightRef() const;
  void setRightRef(const QString& value);

  QString compareMode() const;
  void setCompareMode(const QString& value);

  QString renderer() const;
  void setRenderer(const QString& value);

  QString layoutMode() const;
  void setLayoutMode(const QString& value);

  QVariantList files() const;

  int selectedFileIndex() const;
  void setSelectedFileIndex(int index);
  QObject* selectedFileRowsModel() const;
  int selectedFileRowCount() const;

  QString errorMessage() const;
  bool hasDifftastic() const;

  Q_INVOKABLE bool openRepository(const QString& path);
  Q_INVOKABLE bool chooseRepositoryAndOpen();
  Q_INVOKABLE void compare();
  Q_INVOKABLE void selectFile(int index);
  Q_INVOKABLE QVariantMap selectedFile() const;

 signals:
  void repoPathChanged();
  void refsChanged();
  void leftRefChanged();
  void rightRefChanged();
  void compareModeChanged();
  void rendererChanged();
  void layoutModeChanged();
  void filesChanged();
  void selectedFileIndexChanged();
  void selectedFileChanged();
  void selectedFileRowsChanged();
  void errorMessageChanged();
  void hasDifftasticChanged();

 private:
  void rebuildSelectedFileRows();
  void setError(const QString& error);
  void clearError();
  void persistSettings();

  GitRepositoryService gitService_;
  UnifiedDiffParser parser_;
  BuiltinGitRenderer builtinRenderer_;
  DifftasticRenderer difftasticRenderer_;

  QSettings settings_;

  QString repoPath_;
  QStringList refs_;
  QString leftRef_;
  QString rightRef_;
  QString compareMode_ = "two-dot";
  QString renderer_ = "builtin";
  QString layoutMode_ = "unified";
  QVector<FileDiff> fileDiffs_;
  QVariantList files_;
  DiffRowListModel selectedFileRowsModel_;
  int selectedFileIndex_ = -1;
  QString errorMessage_;
  bool hasDifftastic_ = false;
};

}  // namespace diffy
