#pragma once

#include <QObject>
#include <QSettings>
#include <QStringList>
#include <QVariant>
#include <vector>

#include "app/RepositoryPickerModel.h"
#include <QTimer>

#include "core/CompareSpec.h"
#include "core/GitHubDeviceFlow.h"
#include "core/GitRepositoryService.h"
#include "core/syntax/Highlighter.h"
#include "core/syntax/LanguageRegistry.h"
#include "model/DiffRowListModel.h"
#include "renderers/BuiltinGitRenderer.h"
#include "renderers/DifftasticRenderer.h"

namespace diffy {

class DiffController : public QObject {
  Q_OBJECT
  Q_PROPERTY(QString currentView READ currentView NOTIFY currentViewChanged)
  Q_PROPERTY(QStringList recentRepositories READ recentRepositories NOTIFY recentRepositoriesChanged)
  Q_PROPERTY(QString repoPath READ repoPath WRITE setRepoPath NOTIFY repoPathChanged)
  Q_PROPERTY(QStringList refs READ refs NOTIFY refsChanged)
  Q_PROPERTY(QString leftRef READ leftRef WRITE setLeftRef NOTIFY leftRefChanged)
  Q_PROPERTY(QString rightRef READ rightRef WRITE setRightRef NOTIFY rightRefChanged)
  Q_PROPERTY(QString leftRefDisplay READ leftRefDisplay NOTIFY leftRefChanged)
  Q_PROPERTY(QString rightRefDisplay READ rightRefDisplay NOTIFY rightRefChanged)
  Q_PROPERTY(QString compareMode READ compareMode WRITE setCompareMode NOTIFY compareModeChanged)
  Q_PROPERTY(QString renderer READ renderer WRITE setRenderer NOTIFY rendererChanged)
  Q_PROPERTY(QString layoutMode READ layoutMode WRITE setLayoutMode NOTIFY layoutModeChanged)
  Q_PROPERTY(int compareGeneration READ compareGeneration NOTIFY compareGenerationChanged)
  Q_PROPERTY(QVariantList files READ files NOTIFY filesChanged)
  Q_PROPERTY(int selectedFileIndex READ selectedFileIndex WRITE setSelectedFileIndex NOTIFY selectedFileIndexChanged)
  Q_PROPERTY(QVariantMap selectedFile READ selectedFile NOTIFY selectedFileChanged)
  Q_PROPERTY(QObject* selectedFileRowsModel READ selectedFileRowsModel CONSTANT)
  Q_PROPERTY(int selectedFileRowCount READ selectedFileRowCount NOTIFY selectedFileRowsChanged)
  Q_PROPERTY(bool repositoryPickerVisible READ repositoryPickerVisible NOTIFY repositoryPickerVisibleChanged)
  Q_PROPERTY(QObject* repositoryPickerModel READ repositoryPickerModel CONSTANT)
  Q_PROPERTY(QVariantList branches READ branches NOTIFY branchesChanged)
  Q_PROPERTY(QVariantList commits READ commits NOTIFY commitsChanged)
  Q_PROPERTY(QVariantMap pullRequestInfo READ pullRequestInfo NOTIFY pullRequestInfoChanged)
  Q_PROPERTY(bool comparing READ comparing NOTIFY comparingChanged)
  Q_PROPERTY(bool pullRequestLoading READ pullRequestLoading NOTIFY pullRequestLoadingChanged)
  Q_PROPERTY(QString githubToken READ githubToken WRITE setGithubToken NOTIFY githubTokenChanged)
  Q_PROPERTY(bool hasGithubToken READ hasGithubToken NOTIFY githubTokenChanged)
  Q_PROPERTY(bool oauthInProgress READ oauthInProgress NOTIFY oauthStateChanged)
  Q_PROPERTY(QString oauthUserCode READ oauthUserCode NOTIFY oauthStateChanged)
  Q_PROPERTY(QString oauthVerificationUri READ oauthVerificationUri NOTIFY oauthStateChanged)
  Q_PROPERTY(QString errorMessage READ errorMessage NOTIFY errorMessageChanged)
  Q_PROPERTY(bool wrapEnabled READ wrapEnabled WRITE setWrapEnabled NOTIFY wrapEnabledChanged)
  Q_PROPERTY(int wrapColumn READ wrapColumn WRITE setWrapColumn NOTIFY wrapColumnChanged)
  Q_PROPERTY(bool hasDifftastic READ hasDifftastic NOTIFY hasDifftasticChanged)

 public:
  explicit DiffController(QObject* parent = nullptr);

  QString currentView() const;
  QStringList recentRepositories() const;

  QString repoPath() const;
  void setRepoPath(const QString& path);

  QStringList refs() const;

  QString leftRef() const;
  void setLeftRef(const QString& value);
  QString leftRefDisplay() const;

  QString rightRef() const;
  void setRightRef(const QString& value);
  QString rightRefDisplay() const;

  QString compareMode() const;
  void setCompareMode(const QString& value);

  QString renderer() const;
  void setRenderer(const QString& value);

  QString layoutMode() const;
  void setLayoutMode(const QString& value);
  int compareGeneration() const;

  QVariantList files() const;

  int selectedFileIndex() const;
  void setSelectedFileIndex(int index);
  QObject* selectedFileRowsModel() const;
  int selectedFileRowCount() const;
  bool repositoryPickerVisible() const;
  QObject* repositoryPickerModel() const;

  QVariantList branches() const;
  QVariantList commits() const;
  bool comparing() const;
  QVariantMap pullRequestInfo() const;
  bool pullRequestLoading() const;
  QString githubToken() const;
  void setGithubToken(const QString& token);
  bool hasGithubToken() const;

  bool wrapEnabled() const;
  void setWrapEnabled(bool value);
  int wrapColumn() const;
  void setWrapColumn(int value);

  QString errorMessage() const;
  bool hasDifftastic() const;

  Q_INVOKABLE void goBack();
  Q_INVOKABLE bool openRepository(const QString& path);
  Q_INVOKABLE void openRepositoryPicker();
  Q_INVOKABLE void openRepositoryFromDialog();
  Q_INVOKABLE void closeRepositoryPicker();
  Q_INVOKABLE void navigateRepositoryPickerUp();
  Q_INVOKABLE void activateRepositoryPickerEntry(int index);
  Q_INVOKABLE void openCurrentRepositoryFromPicker();
  Q_INVOKABLE void compare();
  Q_INVOKABLE void selectFile(int index);
  Q_INVOKABLE QVariantMap selectedFile() const;
  Q_INVOKABLE void loadBranches();
  Q_INVOKABLE void loadCommits(const QString& ref);
  Q_INVOKABLE void openPullRequest(const QString& url);
  Q_INVOKABLE QVariantList fuzzyFilter(const QString& query, const QVariantList& items, const QString& labelKey);
  Q_INVOKABLE void startOAuthLogin();
  Q_INVOKABLE void cancelOAuthLogin();

  bool oauthInProgress() const;
  QString oauthUserCode() const;
  QString oauthVerificationUri() const;

 signals:
  void currentViewChanged();
  void recentRepositoriesChanged();
  void repoPathChanged();
  void refsChanged();
  void leftRefChanged();
  void rightRefChanged();
  void compareModeChanged();
  void rendererChanged();
  void layoutModeChanged();
  void compareGenerationChanged();
  void filesChanged();
  void selectedFileIndexChanged();
  void selectedFileChanged();
  void selectedFileRowsChanged();
  void repositoryPickerVisibleChanged();
  void branchesChanged();
  void commitsChanged();
  void pullRequestInfoChanged();
  void comparingChanged();
  void pullRequestLoadingChanged();
  void githubTokenChanged();
  void oauthStateChanged();
  void wrapEnabledChanged();
  void wrapColumnChanged();
  void errorMessageChanged();
  void hasDifftasticChanged();

 private:
  void rebuildSelectedFileRows();
  const std::vector<FlattenedDiffRow>& flattenedRowsForFile(int index);
  void resetFileRowCaches();
  void prefetchFileRows();
  void setCurrentView(const QString& view);
  void addRecentRepository(const QString& path);
  void setError(const QString& error);
  void clearError();
  QString abbreviateRef(const QString& ref) const;
  void persistSettings();

  GitRepositoryService gitService_;
  LanguageRegistry languageRegistry_;
  Highlighter highlighter_;
  BuiltinGitRenderer builtinRenderer_;
  DifftasticRenderer difftasticRenderer_;

  QSettings settings_;

  QString currentView_ = "welcome";
  QStringList recentRepositories_;

  QString repoPath_;
  QStringList refs_;
  QString leftRef_;
  QString rightRef_;
  QString compareMode_ = "two-dot";
  QString renderer_ = "builtin";
  QString layoutMode_ = "unified";
  int compareGeneration_ = 0;
  std::vector<FileDiff> fileDiffs_;
  std::vector<std::vector<FlattenedDiffRow>> flattenedFileRowsCache_;
  std::vector<bool> flattenedFileRowsReady_;
  QVariantList files_;
  DiffRowListModel selectedFileRowsModel_;
  RepositoryPickerModel repositoryPickerModel_;
  bool repositoryPickerVisible_ = false;
  int selectedFileIndex_ = -1;
  QVariantList branches_;
  QVariantList commits_;
  QVariantMap pullRequestInfo_;
  bool comparing_ = false;
  bool pullRequestLoading_ = false;
  QString githubToken_;
  QString errorMessage_;
  bool wrapEnabled_ = false;
  int wrapColumn_ = 0;
  bool hasDifftastic_ = false;
  QString githubClientId_;
  QTimer oauthPollTimer_;
  QString oauthDeviceCode_;
  QString oauthUserCode_;
  QString oauthVerificationUri_;
  int oauthPollInterval_ = 5;
};

}  // namespace diffy
