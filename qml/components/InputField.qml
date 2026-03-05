import QtQuick

Rectangle {
    id: root

    property alias text: input.text
    property alias placeholderText: placeholder.text
    property bool monospace: false
    signal submitted(string value)

    implicitHeight: 40
    radius: 10
    color: "#0f1624"
    border.width: activeFocus || input.activeFocus ? 1 : 0
    border.color: "#4f6998"

    TextInput {
        id: input
        anchors.fill: parent
        anchors.leftMargin: 12
        anchors.rightMargin: 12
        anchors.topMargin: 10
        anchors.bottomMargin: 10
        color: "#e6eeff"
        font.family: root.monospace ? "JetBrains Mono" : "IBM Plex Sans"
        font.pixelSize: 13
        clip: true
        selectByMouse: true
        selectedTextColor: "#0e1522"
        selectionColor: "#8ba8df"
        onAccepted: root.submitted(text)
    }

    Text {
        id: placeholder
        anchors.fill: input
        color: "#61728f"
        font.family: input.font.family
        font.pixelSize: input.font.pixelSize
        verticalAlignment: Text.AlignVCenter
        visible: input.text.length === 0 && !input.activeFocus
    }
}
