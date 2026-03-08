import QtQuick

Item {
    id: root

    property int additions: 0
    property int deletions: 0

    implicitHeight: 4
    implicitWidth: 60

    Rectangle {
        anchors.fill: parent
        radius: 2
        color: theme.panelStrong
        clip: true

        Row {
            anchors.fill: parent

            Rectangle {
                width: root.additions + root.deletions > 0
                       ? parent.width * root.additions / (root.additions + root.deletions)
                       : 0
                height: parent.height
                color: theme.successText
                radius: parent.parent.radius
            }

            Rectangle {
                width: root.additions + root.deletions > 0
                       ? parent.width * root.deletions / (root.additions + root.deletions)
                       : 0
                height: parent.height
                color: theme.dangerText
                radius: parent.parent.radius
            }
        }
    }
}
