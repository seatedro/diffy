#include "core/DiffController.h"

#include <QFileDialog>
#include <QStandardPaths>

#include "core/DiffTypes.h"

namespace diffy {
DiffController::DiffController(QObject* parent)
    : QObject(parent), builtinRenderer_(&parser_), settings_("diffy", "diffy"), selectedFileRowsModel_(this) {
  repoPath_ = settings_.value("repoPath").toString();
  leftRef_ = settings_.value("leftRef").toString();
  rightRef_ = settings_.value("rightRef").toString();
  compareMode_ = settings_.value("compareMode", "two-dot").toString();
  renderer_ = settings_.value("renderer", "builtin").toString();
  layoutMode_ = settings_.value("layoutMode", "unified").toString();

  hasDifftastic_ = !QStandardPaths::findExecutable("difft").isEmpty();

  if (!repoPath_.isEmpty()) {
    QString openError;
    if (gitService_.openRepository(repoPath_, &openError)) {
      refs_ = gitService_.listReferences(nullptr);
      if (leftRef_.isEmpty() && !refs_.isEmpty()) {
        leftRef_ = refs_.first();
      }
      if (rightRef_.isEmpty() && refs_.size() > 1) {
        rightRef_ = refs_.at(1);
      }
    }
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

QString DiffController::errorMessage() const {
  return errorMessage_;
}

bool DiffController::hasDifftastic() const {
  return hasDifftastic_;
}

bool DiffController::openRepository(const QString& path) {
  clearError();

  const bool repoChanged = repoPath_ != path;

  QString error;
  if (!gitService_.openRepository(path, &error)) {
    setError(error);
    return false;
  }

  repoPath_ = path;
  emit repoPathChanged();

  refs_ = gitService_.listReferences(&error);
  if (!error.isEmpty()) {
    setError(error);
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

  persistSettings();
  return true;
}

bool DiffController::chooseRepositoryAndOpen() {
  const QString startPath = repoPath_.isEmpty() ? QDir::homePath() : repoPath_;
  const QString selected = QFileDialog::getExistingDirectory(nullptr, "Open Repository", startPath,
                                                             QFileDialog::ShowDirsOnly | QFileDialog::DontResolveSymlinks);
  if (selected.isEmpty()) {
    return false;
  }
  return openRepository(selected);
}

void DiffController::compare() {
  clearError();

  if (!gitService_.isOpen()) {
    if (repoPath_.isEmpty() || !openRepository(repoPath_)) {
      setError("Open a repository before running compare");
      return;
    }
  }

  QString resolvedLeft;
  QString resolvedRight;
  QString resolveError;

  if (!gitService_.resolveComparison(leftRef_, rightRef_, compareModeFromString(compareMode_), &resolvedLeft,
                                     &resolvedRight, &resolveError)) {
    setError(resolveError);
    return;
  }

  RenderRequest request{repoPath_, resolvedLeft, resolvedRight};
  DiffDocument document;

  IDiffRenderer* renderer = &builtinRenderer_;
  if (renderer_ == "difftastic") {
    renderer = &difftasticRenderer_;
  }

  QString renderError;
  bool rendered = renderer->render(request, &document, &renderError);
  if (!rendered && renderer == &difftasticRenderer_) {
    DiffDocument fallback;
    QString fallbackError;
    if (builtinRenderer_.render(request, &fallback, &fallbackError)) {
      document = fallback;
      setError(QString("difftastic failed (%1). Fell back to built-in renderer.").arg(renderError));
    } else {
      setError(QString("difftastic failed (%1); built-in fallback failed (%2)").arg(renderError, fallbackError));
      return;
    }
  } else if (!rendered) {
    setError(renderError);
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
  if (selectedFileIndex_ >= 0 && selectedFileIndex_ < fileDiffs_.size()) {
    selectedFileRowsModel_.setRows(flattenFileRows(fileDiffs_.at(selectedFileIndex_)));
  } else {
    selectedFileRowsModel_.clear();
  }
  emit selectedFileRowsChanged();
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

void DiffController::persistSettings() {
  settings_.setValue("repoPath", repoPath_);
  settings_.setValue("leftRef", leftRef_);
  settings_.setValue("rightRef", rightRef_);
  settings_.setValue("compareMode", compareMode_);
  settings_.setValue("renderer", renderer_);
  settings_.setValue("layoutMode", layoutMode_);
}

}  // namespace diffy
