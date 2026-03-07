import QtQuick

Rectangle {
    id: root

    property string message: ""
    property string variant: "neutral" // neutral, success, danger
    property int duration: 4000

    signal dismissed()

    function show(msg, v, dur) {
        message = msg
        variant = v || "neutral"
        duration = dur || 4000
        opacity = 1
        dismissTimer.restart()
    }

    function bgColor() {
        if (variant === "success") return theme.successBg
        if (variant === "danger") return theme.dangerBg
        return theme.panelStrong
    }

    function borderCol() {
        if (variant === "success") return theme.successBorder
        if (variant === "danger") return theme.dangerBorder
        return theme.borderStrong
    }

    function textCol() {
        if (variant === "success") return theme.successText
        if (variant === "danger") return theme.dangerText
        return theme.textBase
    }

    width: Math.min(400, toastLabel.implicitWidth + theme.sp8 + closeLabel.implicitWidth + theme.sp2)
    height: toastLabel.implicitHeight + theme.sp4
    radius: theme.radiusLg
    color: bgColor()
    border.color: borderCol()
    border.width: 1
    opacity: 0
    visible: opacity > 0

    Behavior on opacity {
        NumberAnimation { duration: 200; easing.type: Easing.OutCubic }
    }

    Text {
        id: toastLabel
        anchors.left: parent.left
        anchors.right: closeLabel.left
        anchors.verticalCenter: parent.verticalCenter
        anchors.leftMargin: theme.sp3
        anchors.rightMargin: theme.sp2
        text: root.message
        color: root.textCol()
        font.family: theme.sans
        font.pixelSize: theme.fontBody
        elide: Text.ElideRight
    }

    Text {
        id: closeLabel
        anchors.right: parent.right
        anchors.verticalCenter: parent.verticalCenter
        anchors.rightMargin: theme.sp3
        text: "✕"
        color: theme.textFaint
        font.family: theme.sans
        font.pixelSize: theme.fontBody

        MouseArea {
            anchors.fill: parent
            anchors.margins: -theme.sp1
            cursorShape: Qt.PointingHandCursor
            onClicked: {
                root.opacity = 0
                root.dismissed()
            }
        }
    }

    Timer {
        id: dismissTimer
        interval: root.duration
        onTriggered: {
            root.opacity = 0
            root.dismissed()
        }
    }
}
