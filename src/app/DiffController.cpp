#include "app/DiffController.h"

#include <algorithm>

#include <QDir>
#include <QStandardPaths>

#include "app/QtDiffTypes.h"
#include "core/FuzzyMatch.h"
#include "core/GitHubApi.h"
#include "core/GitHubPullRequest.h"
#include "core/DiffTypes.h"
#include "core/Log.h"

namespace diffy {
DiffController::DiffController(QObject* parent)
    : QObject(parent), settings_("diffy", "diffy"), selectedFileRowsModel_(this), repositoryPickerModel_(this) {
  repoPath_ = settings_.value("repoPath").toString();
  leftRef_ = settings_.value("leftRef").toString();
  rightRef_ = settings_.value("rightRef").toString();
  compareMode_ = settings_.value("compareMode", "two-dot").toString();
  renderer_ = settings_.value("renderer", "builtin").toString();
  layoutMode_ = settings_.value("layoutMode", "unified").toString();

  wrapEnabled_ = settings_.value("wrapEnabled", false).toBool();
  wrapColumn_ = settings_.value("wrapColumn", 0).toInt();
  hasDifftastic_ = !QStandardPaths::findExecutable("difft").isEmpty();

  githubToken_ = settings_.value("githubToken").toString();
  if (githubToken_.isEmpty()) {
    const QByteArray envToken = qgetenv("GITHUB_TOKEN");
    if (!envToken.isEmpty()) {
      githubToken_ = QString::fromUtf8(envToken);
    }
  }

  githubClientId_ = "Ov23lijXMwtY1XmHedUM";
  const QByteArray envClientId = qgetenv("DIFFY_GITHUB_CLIENT_ID");
  if (!envClientId.isEmpty()) {
    githubClientId_ = QString::fromUtf8(envClientId);
  }

  connect(&oauthPollTimer_, &QTimer::timeout, this, [this]() {
    const auto response = pollForToken(githubClientId_.toStdString(), oauthDeviceCode_.toStdString());
    switch (response.result) {
      case PollResult::Complete:
        oauthPollTimer_.stop();
        setGithubToken(QString::fromStdString(response.accessToken));
        oauthDeviceCode_.clear();
        oauthUserCode_.clear();
        oauthVerificationUri_.clear();
        emit oauthStateChanged();
        log::info("github-auth", "OAuth login complete");
        break;
      case PollResult::Pending:
        break;
      case PollResult::SlowDown:
        oauthPollInterval_ = std::min(oauthPollInterval_ + 5, 30);
        oauthPollTimer_.setInterval(oauthPollInterval_ * 1000);
        break;
      case PollResult::ExpiredToken:
      case PollResult::Error:
        oauthPollTimer_.stop();
        oauthDeviceCode_.clear();
        oauthUserCode_.clear();
        oauthVerificationUri_.clear();
        emit oauthStateChanged();
        setError(QString::fromStdString(response.error));
        break;
    }
  });

  recentRepositories_ = settings_.value("recentRepositories").toStringList();

  languageRegistry_.loadBuiltinGrammars();
  builtinRenderer_.setSyntax(&languageRegistry_, &highlighter_);

  if (!repoPath_.isEmpty()) {
    std::string openError;
    if (gitService_.openRepository(repoPath_.toStdString(), &openError)) {
      const auto refs = gitService_.listReferences(nullptr);
      refs_.clear();
      for (const std::string& ref : refs) {
        refs_.push_back(QString::fromStdString(ref));
      }
      if (leftRef_.isEmpty() && !refs_.isEmpty()) {
        leftRef_ = refs_.first();
      }
      if (rightRef_.isEmpty() && refs_.size() > 1) {
        rightRef_ = refs_.at(1);
      }
      currentView_ = "compare";
    }
  }
}

QString DiffController::currentView() const {
  return currentView_;
}

QStringList DiffController::recentRepositories() const {
  return recentRepositories_;
}

void DiffController::goBack() {
  if (currentView_ == "diff") {
    setCurrentView("compare");
  } else if (currentView_ == "compare") {
    setCurrentView("welcome");
  }
}

QString DiffController::repoPath() const {
  return repoPath_;
}

void DiffController::setRepoPath(const QString& path) {
  if (repoPath_ == path) {
    return;
  }
  repoPath_ = path;
  emit repoPathChanged();
}

QStringList DiffController::refs() const {
  return refs_;
}

QString DiffController::leftRef() const {
  return leftRef_;
}

void DiffController::setLeftRef(const QString& value) {
  if (leftRef_ == value) {
    return;
  }
  leftRef_ = value;
  emit leftRefChanged();
}

QString DiffController::rightRef() const {
  return rightRef_;
}

void DiffController::setRightRef(const QString& value) {
  if (rightRef_ == value) {
    return;
  }
  rightRef_ = value;
  emit rightRefChanged();
}

QString DiffController::leftRefDisplay() const {
  return abbreviateRef(leftRef_);
}

QString DiffController::rightRefDisplay() const {
  return abbreviateRef(rightRef_);
}

QString DiffController::compareMode() const {
  return compareMode_;
}

void DiffController::setCompareMode(const QString& value) {
  if (compareMode_ == value) {
    return;
  }
  compareMode_ = value;
  emit compareModeChanged();
}

QString DiffController::renderer() const {
  return renderer_;
}

void DiffController::setRenderer(const QString& value) {
  if (renderer_ == value) {
    return;
  }
  renderer_ = value;
  emit rendererChanged();
}

QString DiffController::layoutMode() const {
  return layoutMode_;
}

void DiffController::setLayoutMode(const QString& value) {
  if (layoutMode_ == value) {
    return;
  }
  layoutMode_ = value;
  emit layoutModeChanged();
}

QVariantList DiffController::files() const {
  return files_;
}

int DiffController::selectedFileIndex() const {
  return selectedFileIndex_;
}

void DiffController::setSelectedFileIndex(int index) {
  if (selectedFileIndex_ == index) {
    return;
  }
  selectedFileIndex_ = index;
  emit selectedFileIndexChanged();
  emit selectedFileChanged();
  rebuildSelectedFileRows();
}

QObject* DiffController::selectedFileRowsModel() const {
  return const_cast<DiffRowListModel*>(&selectedFileRowsModel_);
}

int DiffController::selectedFileRowCount() const {
  return selectedFileRowsModel_.count();
}

bool DiffController::comparing() const {
  return comparing_;
}

QVariantList DiffController::branches() const {
  return branches_;
}

QVariantList DiffController::commits() const {
  return commits_;
}

QVariantMap DiffController::pullRequestInfo() const {
  return pullRequestInfo_;
}

bool DiffController::pullRequestLoading() const {
  return pullRequestLoading_;
}

QString DiffController::githubToken() const {
  return githubToken_;
}

void DiffController::setGithubToken(const QString& token) {
  const QString trimmed = token.trimmed();
  if (githubToken_ == trimmed) return;
  githubToken_ = trimmed;
  settings_.setValue("githubToken", githubToken_);
  emit githubTokenChanged();
}

bool DiffController::hasGithubToken() const {
  return !githubToken_.isEmpty();
}

bool DiffController::repositoryPickerVisible() const {
  return repositoryPickerVisible_;
}

QObject* DiffController::repositoryPickerModel() const {
  return const_cast<RepositoryPickerModel*>(&repositoryPickerModel_);
}

bool DiffController::wrapEnabled() const { return wrapEnabled_; }

void DiffController::setWrapEnabled(bool value) {
  if (wrapEnabled_ == value) return;
  wrapEnabled_ = value;
  settings_.setValue("wrapEnabled", wrapEnabled_);
  emit wrapEnabledChanged();
}

int DiffController::wrapColumn() const { return wrapColumn_; }

void DiffController::setWrapColumn(int value) {
  if (wrapColumn_ == value) return;
  wrapColumn_ = value;
  settings_.setValue("wrapColumn", wrapColumn_);
  emit wrapColumnChanged();
}

QString DiffController::errorMessage() const {
  return errorMessage_;
}

bool DiffController::hasDifftastic() const {
  return hasDifftastic_;
}

bool DiffController::openRepository(const QString& path) {
  clearError();

  const bool repoChanged = repoPath_ != path;

  std::string error;
  if (!gitService_.openRepository(path.toStdString(), &error)) {
    setError(QString::fromStdString(error));
    return false;
  }

  repoPath_ = path;
  emit repoPathChanged();

  refs_.clear();
  const auto refs = gitService_.listReferences(&error);
  for (const std::string& ref : refs) {
    refs_.push_back(QString::fromStdString(ref));
  }
  if (!error.empty()) {
    setError(QString::fromStdString(error));
  }
  emit refsChanged();

  if (repoChanged) {
    fileDiffs_.clear();
    files_.clear();
    emit filesChanged();
    selectedFileIndex_ = -1;
    emit selectedFileIndexChanged();
    emit selectedFileChanged();
    rebuildSelectedFileRows();
  }

  if (!refs_.isEmpty()) {
    const QString defaultLeft = refs_.first();
    const QString defaultRight = refs_.size() > 1 ? refs_.at(1) : refs_.first();

    if (repoChanged || leftRef_.isEmpty()) {
      leftRef_ = defaultLeft;
      emit leftRefChanged();
    }
    if (repoChanged || rightRef_.isEmpty()) {
      rightRef_ = defaultRight;
      emit rightRefChanged();
    }
  }

  addRecentRepository(repoPath_);
  loadBranches();
  setCurrentView("compare");
  persistSettings();
  return true;
}

void DiffController::openRepositoryPicker() {
  const QString startPath = repoPath_.isEmpty() ? QDir::homePath() : repoPath_;
  repositoryPickerModel_.setCurrentPath(startPath);
  if (!repositoryPickerVisible_) {
    repositoryPickerVisible_ = true;
    emit repositoryPickerVisibleChanged();
  }
}

void DiffController::closeRepositoryPicker() {
  if (!repositoryPickerVisible_) {
    return;
  }
  repositoryPickerVisible_ = false;
  emit repositoryPickerVisibleChanged();
}

void DiffController::navigateRepositoryPickerUp() {
  repositoryPickerModel_.goUp();
}

void DiffController::activateRepositoryPickerEntry(int index) {
  if (repositoryPickerModel_.entryIsRepository(index)) {
    if (openRepository(repositoryPickerModel_.entryPath(index))) {
      closeRepositoryPicker();
    }
    return;
  }
  repositoryPickerModel_.navigateToEntry(index);
}

void DiffController::openCurrentRepositoryFromPicker() {
  if (!repositoryPickerModel_.currentPathIsRepository()) {
    return;
  }
  if (openRepository(repositoryPickerModel_.currentPath())) {
    closeRepositoryPicker();
  }
}

void DiffController::compare() {
  clearError();
  comparing_ = true;
  emit comparingChanged();

  auto finishComparing = [this]() {
    comparing_ = false;
    emit comparingChanged();
  };

  if (parseGitHubPullRequestUrl(leftRef_.toStdString()).has_value() ||
      parseGitHubPullRequestUrl(rightRef_.toStdString()).has_value()) {
    setError("Use the \"Open PR\" section below to load a pull request URL.");
    finishComparing();
    return;
  }

  if (!gitService_.isOpen()) {
    if (repoPath_.isEmpty() || !openRepository(repoPath_)) {
      setError("Open a repository before running compare");
      finishComparing();
      return;
    }
  }

  std::string resolvedLeft;
  std::string resolvedRight;
  std::string resolveError;

  if (!gitService_.resolveComparison(leftRef_.toStdString(), rightRef_.toStdString(),
                                     compareModeFromString(compareMode_.toStdString()), &resolvedLeft,
                                     &resolvedRight, &resolveError)) {
    setError(QString::fromStdString(resolveError));
    finishComparing();
    return;
  }

  RenderRequest request{repoPath_.toStdString(), resolvedLeft, resolvedRight};
  DiffDocument document;

  IDiffRenderer* renderer = &builtinRenderer_;
  if (renderer_ == "difftastic") {
    renderer = &difftasticRenderer_;
  }

  std::string renderError;
  bool rendered = renderer->render(request, &document, &renderError);
  if (!rendered && renderer == &difftasticRenderer_) {
    DiffDocument fallback;
    std::string fallbackError;
    if (builtinRenderer_.render(request, &fallback, &fallbackError)) {
      document = fallback;
      setError(QString("difftastic failed (%1). Fell back to built-in renderer.")
                   .arg(QString::fromStdString(renderError)));
    } else {
      setError(QString("difftastic failed (%1); built-in fallback failed (%2)")
                   .arg(QString::fromStdString(renderError), QString::fromStdString(fallbackError)));
      finishComparing();
      return;
    }
  } else if (!rendered) {
    setError(QString::fromStdString(renderError));
    finishComparing();
    return;
  }

  fileDiffs_ = document.files;
  files_ = filesToVariantList(fileDiffs_);
  emit filesChanged();

  if (!files_.isEmpty()) {
    selectedFileIndex_ = 0;
  } else {
    selectedFileIndex_ = -1;
  }
  emit selectedFileIndexChanged();
  emit selectedFileChanged();
  rebuildSelectedFileRows();

  finishComparing();
  setCurrentView("diff");
  persistSettings();
}

void DiffController::selectFile(int index) {
  setSelectedFileIndex(index);
}

QVariantMap DiffController::selectedFile() const {
  if (selectedFileIndex_ < 0 || selectedFileIndex_ >= files_.size()) {
    return {};
  }
  return files_.at(selectedFileIndex_).toMap();
}

void DiffController::rebuildSelectedFileRows() {
  if (selectedFileIndex_ >= 0 && selectedFileIndex_ < static_cast<int>(fileDiffs_.size())) {
    selectedFileRowsModel_.setRows(flattenFileRows(fileDiffs_.at(selectedFileIndex_)));
  } else {
    selectedFileRowsModel_.clear();
  }
  emit selectedFileRowsChanged();
}

void DiffController::loadBranches() {
  std::string error;
  const auto branchList = gitService_.listBranches(&error);
  branches_.clear();
  for (const auto& branch : branchList) {
    branches_.append(QVariantMap{
        {"name", QString::fromStdString(branch.name)},
        {"isRemote", branch.isRemote},
        {"isHead", branch.isHead},
    });
  }
  emit branchesChanged();
}

void DiffController::loadCommits(const QString& ref) {
  std::string error;
  const auto commitList = gitService_.listCommits(ref.toStdString(), 100, &error);
  commits_.clear();
  for (const auto& commit : commitList) {
    commits_.append(QVariantMap{
        {"oid", QString::fromStdString(commit.oid)},
        {"summary", QString::fromStdString(commit.summary)},
        {"author", QString::fromStdString(commit.authorName)},
        {"timestamp", static_cast<qint64>(commit.timestamp)},
    });
  }
  emit commitsChanged();
}

void DiffController::openPullRequest(const QString& url) {
  clearError();

  log::info("controller", "openPullRequest: {}", url.toStdString());

  const auto parsed = parseGitHubPullRequestUrl(url.toStdString());
  if (!parsed.has_value()) {
    log::warn("controller", "failed to parse PR URL");
    setError("Not a valid GitHub pull request URL");
    return;
  }

  log::info("controller", "parsed: {}/{} #{}", parsed->owner, parsed->repo, parsed->number);

  pullRequestLoading_ = true;
  emit pullRequestLoadingChanged();

  std::string apiError;
  const auto pr = fetchPullRequest(parsed->owner, parsed->repo, parsed->number,
                                    githubToken_.toStdString(), &apiError);

  pullRequestLoading_ = false;
  emit pullRequestLoadingChanged();

  if (!pr.has_value()) {
    setError(QString::fromStdString(apiError));
    pullRequestInfo_.clear();
    emit pullRequestInfoChanged();
    return;
  }

  pullRequestInfo_ = QVariantMap{
      {"title", QString::fromStdString(pr->title)},
      {"baseBranch", QString::fromStdString(pr->baseBranch)},
      {"headBranch", QString::fromStdString(pr->headBranch)},
      {"baseSha", QString::fromStdString(pr->baseSha)},
      {"headSha", QString::fromStdString(pr->headSha)},
      {"state", QString::fromStdString(pr->state)},
      {"author", QString::fromStdString(pr->authorLogin)},
      {"number", pr->number},
      {"additions", pr->additions},
      {"deletions", pr->deletions},
      {"changedFiles", pr->changedFiles},
  };
  emit pullRequestInfoChanged();

  if (!gitService_.isOpen()) {
    setError("Open a local clone of " + QString::fromStdString(parsed->owner) + "/" +
             QString::fromStdString(parsed->repo) + " first, then try the PR URL again.");
    return;
  }

  log::info("controller", "fetching PR refs from remote for #{}", parsed->number);

  std::string resolvedLeft;
  std::string resolvedRight;
  std::string resolveError;
  if (!gitService_.resolvePullRequestComparison(url.toStdString(), &resolvedLeft, &resolvedRight, &resolveError)) {
    log::error("controller", "failed to resolve PR comparison: {}", resolveError);
    setError(QString::fromStdString(resolveError));
    return;
  }

  leftRef_ = QString::fromStdString(resolvedLeft);
  emit leftRefChanged();
  rightRef_ = QString::fromStdString(resolvedRight);
  emit rightRefChanged();

  if (compareMode_ != "three-dot") {
    compareMode_ = "three-dot";
    emit compareModeChanged();
  }

  compare();
}

QVariantList DiffController::fuzzyFilter(const QString& query, const QVariantList& items, const QString& labelKey) {
  if (query.isEmpty()) return items;

  std::vector<std::string> labels;
  labels.reserve(items.size());
  for (const auto& item : items) {
    labels.push_back(item.toMap().value(labelKey).toString().toStdString());
  }

  const auto ranked = fuzzyRank(query.toStdString(), labels);
  QVariantList result;
  result.reserve(ranked.size());
  for (const auto& r : ranked) {
    result.append(items.at(r.index));
  }
  return result;
}

void DiffController::startOAuthLogin() {
  clearError();

  if (githubClientId_.isEmpty()) {
    setError("Set DIFFY_GITHUB_CLIENT_ID or configure a GitHub OAuth App client ID first.");
    return;
  }

  std::string flowError;
  const auto deviceCode = requestDeviceCode(githubClientId_.toStdString(), "repo", &flowError);
  if (!deviceCode.has_value()) {
    setError(QString::fromStdString(flowError));
    return;
  }

  oauthDeviceCode_ = QString::fromStdString(deviceCode->deviceCode);
  oauthUserCode_ = QString::fromStdString(deviceCode->userCode);
  oauthVerificationUri_ = QString::fromStdString(deviceCode->verificationUri);
  oauthPollInterval_ = deviceCode->interval;
  emit oauthStateChanged();

  oauthPollTimer_.start(oauthPollInterval_ * 1000);
}

void DiffController::cancelOAuthLogin() {
  oauthPollTimer_.stop();
  oauthDeviceCode_.clear();
  oauthUserCode_.clear();
  oauthVerificationUri_.clear();
  emit oauthStateChanged();
}

bool DiffController::oauthInProgress() const {
  return !oauthDeviceCode_.isEmpty();
}

QString DiffController::oauthUserCode() const {
  return oauthUserCode_;
}

QString DiffController::oauthVerificationUri() const {
  return oauthVerificationUri_;
}

void DiffController::setError(const QString& error) {
  errorMessage_ = error;
  emit errorMessageChanged();
}

void DiffController::clearError() {
  if (errorMessage_.isEmpty()) {
    return;
  }
  errorMessage_.clear();
  emit errorMessageChanged();
}

void DiffController::setCurrentView(const QString& view) {
  if (currentView_ == view) {
    return;
  }
  currentView_ = view;
  emit currentViewChanged();
}

void DiffController::addRecentRepository(const QString& path) {
  recentRepositories_.removeAll(path);
  recentRepositories_.prepend(path);
  constexpr int kMaxRecents = 10;
  while (recentRepositories_.size() > kMaxRecents) {
    recentRepositories_.removeLast();
  }
  settings_.setValue("recentRepositories", recentRepositories_);
  emit recentRepositoriesChanged();
}

QString DiffController::abbreviateRef(const QString& ref) const {
  if (ref.size() != 40) {
    return ref;
  }
  const QByteArray ascii = ref.toLatin1();
  const bool allHex = std::all_of(ascii.begin(), ascii.end(), [](char c) {
    return (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f') || (c >= 'A' && c <= 'F');
  });
  if (!allHex) {
    return ref;
  }
  const std::string branchName = gitService_.resolveOidToBranchName(ref.toStdString());
  if (!branchName.empty()) {
    return QString::fromStdString(branchName);
  }
  return ref.left(8);
}

void DiffController::persistSettings() {
  settings_.setValue("repoPath", repoPath_);
  settings_.setValue("leftRef", leftRef_);
  settings_.setValue("rightRef", rightRef_);
  settings_.setValue("compareMode", compareMode_);
  settings_.setValue("renderer", renderer_);
  settings_.setValue("layoutMode", layoutMode_);
}

}  // namespace diffy
