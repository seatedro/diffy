#pragma once

#include <QVariant>

#include "core/diff/DiffTypes.h"

namespace diffy {

QString lineKindToQString(LineKind kind);
QVariantMap lineToVariant(const DiffLine& line);
QVariantMap hunkToVariant(const Hunk& hunk);
QVariantMap fileDiffToVariant(const FileDiff& file);
QVariantList filesToVariantList(const std::vector<FileDiff>& files);

}  // namespace diffy
