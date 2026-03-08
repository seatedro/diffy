#pragma once

#include <QAbstractListModel>
#include <QString>
#include <vector>

namespace diffy {

struct RepositoryPickerEntry {
  QString name;
  QString path;
  bool isRepository = false;
};

class RepositoryPickerModel : public QAbstractListModel {
  Q_OBJECT
  Q_PROPERTY(QString currentPath READ currentPath NOTIFY currentPathChanged)
  Q_PROPERTY(bool currentPathIsRepository READ currentPathIsRepository NOTIFY currentPathIsRepositoryChanged)

 public:
  enum Role {
    NameRole = Qt::UserRole + 1,
    PathRole,
    IsRepositoryRole,
  };

  explicit RepositoryPickerModel(QObject* parent = nullptr);
  ~RepositoryPickerModel() override;

  int rowCount(const QModelIndex& parent = QModelIndex()) const override;
  QVariant data(const QModelIndex& index, int role) const override;
  QHash<int, QByteArray> roleNames() const override;

  QString currentPath() const;
  bool currentPathIsRepository() const;

  void setCurrentPath(const QString& path);
  bool goUp();
  bool navigateToEntry(int index);
  QString entryPath(int index) const;
  bool entryIsRepository(int index) const;

 signals:
  void currentPathChanged();
  void currentPathIsRepositoryChanged();

 private:
  void reload();
  static bool isRepositoryRoot(const QString& path);

  QString currentPath_;
  bool currentPathIsRepository_ = false;
  std::vector<RepositoryPickerEntry> entries_;
};

}  // namespace diffy
