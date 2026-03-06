import QtQuick

Rectangle {
    id: root

    required property QtObject theme
    property alias text: input.text
    property alias placeholderText: placeholder.text
    property bool monospace: false
    signal submitted(string value)

    implicitHeight: 30
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
        anchors.topMargin: 5
        anchors.bottomMargin: 5
        color: theme.textStrong
        font.family: root.monospace ? theme.mono : theme.sans
        font.pixelSize: 11
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
