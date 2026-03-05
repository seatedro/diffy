#pragma once

#include "renderers/IDiffRenderer.h"

namespace diffy {

class DifftasticRenderer : public IDiffRenderer {
 public:
  QString id() const override;
  bool render(const RenderRequest& request, DiffDocument* out, QString* error) override;

 private:
  bool parseDifftasticJson(const QByteArray& json,
                          const QString& fallbackPath,
                          const QString& fallbackStatus,
                          FileDiff* outFile,
                          QString* error) const;
};

}  // namespace diffy
