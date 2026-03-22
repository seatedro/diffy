import QtQuick
import QtQuick.Window

Rectangle {
    id: root

    property alias text: label.text
    property string tone: "neutral"
    property bool active: false
    property bool compact: false
    property string toolTip: ""
    signal clicked()

    function fillColor() {
        if (tone === "accent") return active ? theme.accent : theme.accentSoft
        if (tone === "success") return theme.successBg
        if (tone === "danger") return theme.dangerBg
        return mouseArea.containsMouse ? theme.panelStrong : theme.panel
    }

    function borderColor() {
        if (tone === "accent") return active ? theme.accent : theme.selectionBorder
        if (tone === "success") return theme.successBorder
        if (tone === "danger") return theme.dangerBorder
        return active ? theme.selectionBorder : theme.borderSoft
    }

    function textColor() {
        if (tone === "accent") return active ? theme.appBg : theme.accentStrong
        if (tone === "success") return theme.successText
        if (tone === "danger") return theme.dangerText
        return active ? theme.accentStrong : theme.textMuted
    }

    activeFocusOnTab: true

    implicitWidth: Math.max(compact ? 58 : 88, label.implicitWidth + (compact ? theme.sp4 : theme.sp6))
    implicitHeight: compact ? 28 : 32
    radius: theme.radiusSm
    color: mouseArea.pressed ? Qt.darker(fillColor(), 1.1) : (mouseArea.containsMouse ? Qt.lighter(fillColor(), 1.04) : fillColor())
    border.width: active ? 1.5 : 1
    border.color: borderColor()

    scale: mouseArea.pressed ? 0.97 : 1.0
    Behavior on scale {
        enabled: !(Window.window && Window.window.commandPaletteShowing)
        NumberAnimation { duration: 80; easing.type: Easing.OutCubic }
    }

    Behavior on color {
        enabled: !(Window.window && Window.window.commandPaletteShowing)
        ColorAnimation { duration: 90 }
    }

    Text {
        id: label
        anchors.centerIn: parent
        color: root.textColor()
        font.family: theme.sans
        font.pixelSize: compact ? theme.fontSmall + 1 : theme.fontBody
        font.bold: active || tone !== "neutral"
    }

    // Focus ring
    Rectangle {
        anchors.fill: parent
        anchors.margins: -2
        radius: root.radius + 2
        color: "transparent"
        border.width: 2
        border.color: theme.accent
        visible: root.activeFocus
    }

    MouseArea {
        id: mouseArea
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: root.clicked()
    }

    Keys.onReturnPressed: root.clicked()
    Keys.onSpacePressed: root.clicked()

    // Hand-rolled tooltip
    Rectangle {
        id: tipBg
        visible: root.toolTip.length > 0 && tipShown
        x: (root.width - width) / 2
        y: root.height + theme.sp1
        width: tipLabel.implicitWidth + theme.sp3
        height: tipLabel.implicitHeight + theme.sp1
        radius: theme.radiusSm
        color: theme.panelStrong
        border.color: theme.borderSoft
        z: 100

        property bool tipShown: false

        Text {
            id: tipLabel
            anchors.centerIn: parent
            text: root.toolTip
            color: theme.textBase
            font.family: theme.sans
            font.pixelSize: theme.fontSmall
        }
    }

    Timer {
        id: tipTimer
        interval: 600
        repeat: false
        onTriggered: tipBg.tipShown = true
    }

    Connections {
        target: mouseArea
        function onContainsMouseChanged() {
            if (mouseArea.containsMouse) {
                tipTimer.start()
            } else {
                tipTimer.stop()
                tipBg.tipShown = false
            }
        }
    }
}
