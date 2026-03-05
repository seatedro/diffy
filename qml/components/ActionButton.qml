import QtQuick

Rectangle {
    id: root

    required property QtObject theme
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
        return active ? theme.accentSoft : theme.panel
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
            return active ? "#ffffff" : theme.accentStrong
        }
        if (tone === "success") {
            return theme.successText
        }
        if (tone === "danger") {
            return theme.dangerText
        }
        return active ? theme.accentStrong : theme.textBase
    }

    implicitWidth: Math.max(compact ? 78 : 100, label.implicitWidth + (compact ? 22 : 30))
    implicitHeight: compact ? 32 : 36
    radius: 6
    color: mouseArea.containsMouse ? Qt.lighter(fillColor(), 1.03) : fillColor()
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
        font.pixelSize: compact ? 12 : 13
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
