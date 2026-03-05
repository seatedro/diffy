#pragma once

#include <QString>
#include <QStringList>

#include <git2.h>

#include "core/CompareSpec.h"

namespace diffy {

class GitRepositoryService {
 public:
  GitRepositoryService();
  ~GitRepositoryService();

  GitRepositoryService(const GitRepositoryService&) = delete;
  GitRepositoryService& operator=(const GitRepositoryService&) = delete;

  bool openRepository(const QString& path, QString* error);
  bool isOpen() const;
  QString repositoryPath() const;

  QStringList listReferences(QString* error) const;

  bool resolveComparison(const QString& leftRef,
                         const QString& rightRef,
                         CompareMode mode,
                         QString* outLeftRevision,
                         QString* outRightRevision,
                         QString* error) const;

 private:
  git_repository* repo_ = nullptr;
  QString repositoryPath_;
};

}  // namespace diffy
