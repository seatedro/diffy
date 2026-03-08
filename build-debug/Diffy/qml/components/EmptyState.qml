import QtQuick
import QtQuick.Layouts

Item {
    id: root

    property string icon: ""
    property string title: ""
    property string subtitle: ""

    implicitWidth: col.implicitWidth
    implicitHeight: col.implicitHeight

    ColumnLayout {
        id: col
        anchors.centerIn: parent
        spacing: theme.sp2

        Text {
            visible: root.icon.length > 0
            Layout.alignment: Qt.AlignHCenter
            text: root.icon
            color: theme.textFaint
            font.pixelSize: 32
        }

        Text {
            visible: root.title.length > 0
            Layout.alignment: Qt.AlignHCenter
            text: root.title
            color: theme.textStrong
            font.family: theme.sans
            font.pixelSize: theme.fontTitle
            font.bold: true
        }

        Text {
            visible: root.subtitle.length > 0
            Layout.alignment: Qt.AlignHCenter
            text: root.subtitle
            color: theme.textMuted
            font.family: theme.sans
            font.pixelSize: theme.fontBody
        }
    }
}
