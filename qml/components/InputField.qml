import QtQuick

Rectangle {
    id: root

    property alias text: input.text
    property alias placeholderText: placeholder.text
    property bool monospace: false
    property bool compact: false
    signal submitted(string value)

    implicitHeight: compact ? 26 : 30
    radius: 4
    color: theme.panelStrong
    border.width: 1
    border.color: input.activeFocus ? theme.selectionBorder : theme.borderSoft

    Behavior on border.color {
        ColorAnimation { duration: 90 }
    }

    TextInput {
        id: input
        anchors.fill: parent
        anchors.leftMargin: 12
        anchors.rightMargin: 12
        anchors.topMargin: root.compact ? 4 : 5
        anchors.bottomMargin: root.compact ? 4 : 5
        color: theme.textStrong
        font.family: root.monospace ? theme.mono : theme.sans
        font.pixelSize: root.compact ? 10 : 11
        clip: true
        selectByMouse: true
        selectedTextColor: "#ffffff"
        selectionColor: theme.accent
        onAccepted: root.submitted(text)
    }

    Text {
        id: placeholder
        anchors.fill: input
        color: theme.textFaint
        font.family: input.font.family
        font.pixelSize: input.font.pixelSize
        verticalAlignment: Text.AlignVCenter
        visible: input.text.length === 0 && !input.activeFocus
    }
}
