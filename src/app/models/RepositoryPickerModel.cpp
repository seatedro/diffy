#include "app/models/RepositoryPickerModel.h"

#include <QDir>
#include <QFileInfo>

#include <git2.h>

namespace diffy {

RepositoryPickerModel::RepositoryPickerModel(QObject* parent) : QAbstractListModel(parent) {
  git_libgit2_init();
}

RepositoryPickerModel::~RepositoryPickerModel() {
  git_libgit2_shutdown();
}

int RepositoryPickerModel::rowCount(const QModelIndex& parent) const {
  if (parent.isValid()) {
    return 0;
  }
  return static_cast<int>(entries_.size());
}

QVariant RepositoryPickerModel::data(const QModelIndex& index, int role) const {
  if (!index.isValid() || index.row() < 0 || index.row() >= static_cast<int>(entries_.size())) {
    return {};
  }

  const RepositoryPickerEntry& entry = entries_.at(static_cast<size_t>(index.row()));
  switch (role) {
    case NameRole:
      return entry.name;
    case PathRole:
      return entry.path;
    case IsRepositoryRole:
      return entry.isRepository;
    default:
      return {};
  }
}

QHash<int, QByteArray> RepositoryPickerModel::roleNames() const {
  return {
      {NameRole, "name"},
      {PathRole, "path"},
      {IsRepositoryRole, "isRepository"},
  };
}

QString RepositoryPickerModel::currentPath() const {
  return currentPath_;
}

bool RepositoryPickerModel::currentPathIsRepository() const {
  return currentPathIsRepository_;
}

void RepositoryPickerModel::setCurrentPath(const QString& path) {
  const QString normalized = QDir(path).absolutePath();
  if (currentPath_ == normalized && !entries_.empty()) {
    return;
  }
  currentPath_ = normalized;
  reload();
  emit currentPathChanged();
}

bool RepositoryPickerModel::goUp() {
  if (currentPath_.isEmpty()) {
    return false;
  }
  QDir dir(currentPath_);
  if (!dir.cdUp()) {
    return false;
  }
  setCurrentPath(dir.absolutePath());
  return true;
}

bool RepositoryPickerModel::navigateToEntry(int index) {
  if (index < 0 || index >= static_cast<int>(entries_.size())) {
    return false;
  }
  setCurrentPath(entries_.at(static_cast<size_t>(index)).path);
  return true;
}

QString RepositoryPickerModel::entryPath(int index) const {
  if (index < 0 || index >= static_cast<int>(entries_.size())) {
    return {};
  }
  return entries_.at(static_cast<size_t>(index)).path;
}

bool RepositoryPickerModel::entryIsRepository(int index) const {
  if (index < 0 || index >= static_cast<int>(entries_.size())) {
    return false;
  }
  return entries_.at(static_cast<size_t>(index)).isRepository;
}

void RepositoryPickerModel::reload() {
  beginResetModel();
  entries_.clear();

  QDir dir(currentPath_);
  const QFileInfoList infos =
      dir.entryInfoList(QDir::Dirs | QDir::NoDotAndDotDot | QDir::Readable, QDir::DirsFirst | QDir::IgnoreCase);

  for (const QFileInfo& info : infos) {
    RepositoryPickerEntry entry;
    entry.name = info.fileName();
    entry.path = info.absoluteFilePath();
    entry.isRepository = isRepositoryRoot(entry.path);
    entries_.push_back(std::move(entry));
  }
  endResetModel();

  const bool nextIsRepository = isRepositoryRoot(currentPath_);
  if (currentPathIsRepository_ != nextIsRepository) {
    currentPathIsRepository_ = nextIsRepository;
    emit currentPathIsRepositoryChanged();
  }
}

bool RepositoryPickerModel::isRepositoryRoot(const QString& path) {
  git_repository* repo = nullptr;
  const QByteArray pathUtf8 = path.toUtf8();
  const int result =
      git_repository_open_ext(&repo, pathUtf8.constData(), GIT_REPOSITORY_OPEN_NO_SEARCH, nullptr);
  if (result == 0 && repo != nullptr) {
    git_repository_free(repo);
    return true;
  }
  return false;
}

}  // namespace diffy
