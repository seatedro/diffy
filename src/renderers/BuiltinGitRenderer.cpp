#include "renderers/BuiltinGitRenderer.h"

#include <QProcess>

namespace diffy {

BuiltinGitRenderer::BuiltinGitRenderer(const UnifiedDiffParser* parser) : parser_(parser) {}

QString BuiltinGitRenderer::id() const {
  return "builtin";
}

bool BuiltinGitRenderer::render(const RenderRequest& request, DiffDocument* out, QString* error) {
  if (parser_ == nullptr) {
    if (error) {
      *error = "Renderer parser is not configured";
    }
    return false;
  }

  QProcess process;
  process.setProgram("git");
  process.setArguments({"-C", request.repoPath, "diff", "--no-color", "--unified=3", request.leftRevision,
                        request.rightRevision});
  process.start();

  if (!process.waitForFinished(120000)) {
    if (error) {
      *error = "Timed out while running git diff";
    }
    return false;
  }

  const QByteArray stdoutData = process.readAllStandardOutput();
  const QByteArray stderrData = process.readAllStandardError();

  if (process.exitStatus() != QProcess::NormalExit || process.exitCode() > 1) {
    if (error) {
      *error = QString("git diff failed: %1").arg(QString::fromUtf8(stderrData).trimmed());
    }
    return false;
  }

  *out = parser_->parse(request.leftRevision, request.rightRevision, QString::fromUtf8(stdoutData));
  return true;
}

}  // namespace diffy
