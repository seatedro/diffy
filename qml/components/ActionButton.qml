import QtQuick

Rectangle {
    id: root

    property alias text: label.text
    property string tone: "neutral"
    property bool active: false
    property bool compact: false
    signal clicked()

    function fillColor() {
        if (tone === "accent") {
            return active ? theme.accent : theme.accentSoft
        }
        if (tone === "success") {
            return theme.successBg
        }
        if (tone === "danger") {
            return theme.dangerBg
        }
        return mouseArea.containsMouse ? theme.panelStrong : theme.panel
    }

    function borderColor() {
        if (tone === "accent") {
            return active ? theme.accent : theme.selectionBorder
        }
        if (tone === "success") {
            return theme.successBorder
        }
        if (tone === "danger") {
            return theme.dangerBorder
        }
        return active ? theme.selectionBorder : theme.borderSoft
    }

    function textColor() {
        if (tone === "accent") {
            return active ? theme.appBg : theme.accentStrong
        }
        if (tone === "success") {
            return theme.successText
        }
        if (tone === "danger") {
            return theme.dangerText
        }
        return active ? theme.accentStrong : theme.textMuted
    }

    implicitWidth: Math.max(compact ? 58 : 88, label.implicitWidth + (compact ? 14 : 24))
    implicitHeight: compact ? 26 : 28
    radius: 4
    color: mouseArea.containsMouse ? Qt.lighter(fillColor(), 1.04) : fillColor()
    border.width: 1
    border.color: borderColor()

    Behavior on color {
        ColorAnimation { duration: 90 }
    }

    Text {
        id: label
        anchors.centerIn: parent
        color: root.textColor()
        font.family: theme.sans
        font.pixelSize: compact ? 10 : 11
        font.bold: active || tone !== "neutral"
    }

    MouseArea {
        id: mouseArea
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: root.clicked()
    }
}
