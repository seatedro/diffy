import QtQuick
import QtQuick.Layouts

Rectangle {
    id: root

    property var options: []  // [{label: "Unified", value: "unified"}, ...]
    property string currentValue: ""
    signal valueChanged(string value)

    activeFocusOnTab: true

    implicitWidth: row.implicitWidth + theme.sp1
    implicitHeight: 28
    radius: theme.radiusMd
    color: theme.panelStrong
    border.color: theme.borderSoft

    Row {
        id: row
        anchors.centerIn: parent
        spacing: 1

        Repeater {
            model: root.options

            Rectangle {
                required property var modelData
                required property int index

                width: segLabel.implicitWidth + theme.sp4
                height: root.height - theme.sp1
                radius: theme.radiusSm
                color: root.currentValue === modelData.value
                    ? theme.accent
                    : (segMouse.containsMouse ? theme.panelTint : "transparent")

                Behavior on color { ColorAnimation { duration: 35 } }

                Text {
                    id: segLabel
                    anchors.centerIn: parent
                    text: modelData.label
                    color: root.currentValue === modelData.value ? theme.appBg : theme.textMuted
                    font.family: theme.sans
                    font.pixelSize: theme.fontSmall
                    font.bold: root.currentValue === modelData.value
                }

                MouseArea {
                    id: segMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: root.valueChanged(modelData.value)
                }
            }
        }
    }

    Rectangle {
        anchors.fill: parent
        anchors.margins: -2
        radius: root.radius + 2
        color: "transparent"
        border.width: 2
        border.color: theme.accent
        visible: root.activeFocus
    }
}
