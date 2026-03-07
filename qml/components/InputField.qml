import QtQuick

Rectangle {
    id: root

    property alias text: input.text
    property alias placeholderText: placeholder.text
    property bool monospace: false
    property bool compact: false
    property bool error: false
    signal submitted(string value)

    implicitHeight: compact ? 26 : 30
    radius: theme.radiusSm
    color: theme.panelStrong
    border.width: input.activeFocus ? 1.5 : 1
    border.color: root.error ? theme.dangerBorder : (input.activeFocus ? theme.accent : theme.borderSoft)

    Behavior on border.color { ColorAnimation { duration: 90 } }
    Behavior on border.width { NumberAnimation { duration: 90 } }

    TextInput {
        id: input
        anchors.fill: parent
        anchors.leftMargin: theme.sp3
        anchors.rightMargin: theme.sp3
        anchors.topMargin: root.compact ? theme.sp1 : 5
        anchors.bottomMargin: root.compact ? theme.sp1 : 5
        color: theme.textStrong
        font.family: root.monospace ? theme.mono : theme.sans
        font.pixelSize: root.compact ? theme.fontSmall + 1 : theme.fontBody
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
