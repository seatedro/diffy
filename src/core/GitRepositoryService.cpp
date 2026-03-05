#include "core/GitRepositoryService.h"

#include <QSet>

namespace diffy {
namespace {

QString lastGitError(const QString& fallback) {
  if (const git_error* err = git_error_last(); err && err->message) {
    return QString::fromUtf8(err->message);
  }
  return fallback;
}

QString oidToString(const git_oid& oid) {
  char out[GIT_OID_HEXSZ + 1] = {0};
  git_oid_fmt(out, &oid);
  out[GIT_OID_HEXSZ] = '\0';
  return QString::fromLatin1(out);
}

bool resolveToCommitOid(git_repository* repo, const QString& ref, git_oid* out, QString* error) {
  git_object* object = nullptr;
  git_object* peeled = nullptr;

  const QByteArray refUtf8 = ref.toUtf8();
  if (git_revparse_single(&object, repo, refUtf8.constData()) != 0) {
    if (error) {
      *error = lastGitError(QString("Failed to resolve reference: %1").arg(ref));
    }
    return false;
  }

  if (git_object_peel(&peeled, object, GIT_OBJECT_COMMIT) != 0) {
    git_object_free(object);
    if (error) {
      *error = lastGitError(QString("Reference is not a commit: %1").arg(ref));
    }
    return false;
  }

  const git_oid* oid = git_object_id(peeled);
  git_oid_cpy(out, oid);

  git_object_free(peeled);
  git_object_free(object);
  return true;
}

}  // namespace

GitRepositoryService::GitRepositoryService() {
  git_libgit2_init();
}

GitRepositoryService::~GitRepositoryService() {
  if (repo_ != nullptr) {
    git_repository_free(repo_);
    repo_ = nullptr;
  }
  git_libgit2_shutdown();
}

bool GitRepositoryService::openRepository(const QString& path, QString* error) {
  if (repo_ != nullptr) {
    git_repository_free(repo_);
    repo_ = nullptr;
  }

  const QByteArray pathUtf8 = path.toUtf8();
  if (git_repository_open_ext(&repo_, pathUtf8.constData(), 0, nullptr) != 0) {
    if (error) {
      *error = lastGitError(QString("Failed to open repository: %1").arg(path));
    }
    return false;
  }

  repositoryPath_ = path;
  return true;
}

bool GitRepositoryService::isOpen() const {
  return repo_ != nullptr;
}

QString GitRepositoryService::repositoryPath() const {
  return repositoryPath_;
}

QStringList GitRepositoryService::listReferences(QString* error) const {
  QStringList refs;
  if (repo_ == nullptr) {
    if (error) {
      *error = "Repository is not open";
    }
    return refs;
  }

  git_reference_iterator* iterator = nullptr;
  if (git_reference_iterator_new(&iterator, repo_) != 0) {
    if (error) {
      *error = lastGitError("Failed to iterate references");
    }
    return refs;
  }

  QSet<QString> uniqueRefs;
  git_reference* reference = nullptr;
  while (git_reference_next(&reference, iterator) == 0) {
    const char* shorthand = git_reference_shorthand(reference);
    if (shorthand != nullptr) {
      uniqueRefs.insert(QString::fromUtf8(shorthand));
    }
    git_reference_free(reference);
  }

  git_reference_iterator_free(iterator);

  refs = uniqueRefs.values();
  refs.sort();
  return refs;
}

bool GitRepositoryService::resolveComparison(const QString& leftRef,
                                             const QString& rightRef,
                                             CompareMode mode,
                                             QString* outLeftRevision,
                                             QString* outRightRevision,
                                             QString* error) const {
  if (repo_ == nullptr) {
    if (error) {
      *error = "Repository is not open";
    }
    return false;
  }

  git_oid leftOid{};
  git_oid rightOid{};

  if (mode == CompareMode::SingleCommit) {
    const QString commitRef = rightRef.isEmpty() ? leftRef : rightRef;
    if (commitRef.isEmpty()) {
      if (error) {
        *error = "Single-commit mode requires a commit reference";
      }
      return false;
    }

    if (!resolveToCommitOid(repo_, commitRef, &rightOid, error)) {
      return false;
    }

    git_commit* commit = nullptr;
    if (git_commit_lookup(&commit, repo_, &rightOid) != 0) {
      if (error) {
        *error = lastGitError("Failed to load commit");
      }
      return false;
    }

    if (git_commit_parentcount(commit) == 0) {
      git_commit_free(commit);
      if (error) {
        *error = "Cannot diff the root commit in single-commit mode yet";
      }
      return false;
    }

    const git_oid* parentOid = git_commit_parent_id(commit, 0);
    git_oid_cpy(&leftOid, parentOid);
    git_commit_free(commit);

    if (outLeftRevision) {
      *outLeftRevision = oidToString(leftOid);
    }
    if (outRightRevision) {
      *outRightRevision = oidToString(rightOid);
    }
    return true;
  }

  if (leftRef.isEmpty() || rightRef.isEmpty()) {
    if (error) {
      *error = "Comparison requires both left and right references";
    }
    return false;
  }

  if (!resolveToCommitOid(repo_, leftRef, &leftOid, error)) {
    return false;
  }
  if (!resolveToCommitOid(repo_, rightRef, &rightOid, error)) {
    return false;
  }

  if (mode == CompareMode::ThreeDot) {
    git_oid baseOid{};
    if (git_merge_base(&baseOid, repo_, &leftOid, &rightOid) != 0) {
      if (error) {
        *error = lastGitError("Failed to resolve merge base");
      }
      return false;
    }
    leftOid = baseOid;
  }

  if (outLeftRevision) {
    *outLeftRevision = oidToString(leftOid);
  }
  if (outRightRevision) {
    *outRightRevision = oidToString(rightOid);
  }
  return true;
}

}  // namespace diffy
