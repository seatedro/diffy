import QtQuick

Rectangle {
    id: root

    property alias text: label.text
    property color fillColor: "#22314f"
    property color hoverColor: "#2c4169"
    property color textColor: "#e6eeff"
    property bool emphasized: false
    signal clicked()

    implicitWidth: Math.max(100, label.implicitWidth + 28)
    implicitHeight: 38
    radius: 10
    color: mouseArea.containsMouse ? hoverColor : fillColor
    border.width: emphasized ? 0 : 1
    border.color: "#334766"

    Behavior on color {
        ColorAnimation { duration: 120 }
    }

    Text {
        id: label
        anchors.centerIn: parent
        color: root.textColor
        font.family: "IBM Plex Sans"
        font.pixelSize: 13
        font.bold: emphasized
    }

    MouseArea {
        id: mouseArea
        anchors.fill: parent
        hoverEnabled: true
        onClicked: root.clicked()
    }
}
