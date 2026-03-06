#pragma once

#include <QByteArray>
#include <QString>

#include "renderers/IDiffRenderer.h"

namespace diffy {

class DifftasticRenderer : public IDiffRenderer {
 public:
  std::string_view id() const override;
  bool render(const RenderRequest& request, DiffDocument* out, std::string* error) override;

 private:
  bool parseDifftasticJson(const QByteArray& json,
                          const QString& fallbackPath,
                          const QString& fallbackStatus,
                          FileDiff* outFile,
                          std::string* error) const;
};

}  // namespace diffy
