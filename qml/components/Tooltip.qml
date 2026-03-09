import QtQuick

Item {
    id: root

    property string text: ""
    property Item target: null
    property int delay: 500
    property int hideDelay: 100
    property string position: "bottom"
    property bool showing: false

    property bool shouldShow: false
    visible: false
    z: 1000

    Timer {
        id: showTimer
        interval: root.delay
        repeat: false
        onTriggered: {
            if (root.shouldShow && root.text.length > 0) {
                root.positionTooltip()
                root.visible = true
                root.showing = true
            }
        }
    }

    Timer {
        id: hideTimer
        interval: root.hideDelay
        repeat: false
        onTriggered: {
            if (!root.shouldShow) {
                root.visible = false
                root.showing = false
            }
        }
    }

    function show() {
        shouldShow = true
        hideTimer.stop()
        showTimer.restart()
    }

    function hide() {
        shouldShow = false
        showTimer.stop()
        hideTimer.restart()
    }

    function positionTooltip() {
        if (!target) return
        var globalPos = target.mapToItem(null, 0, 0)
        var targetW = target.width
        var targetH = target.height
        var tipW = tooltipBg.width
        var tipH = tooltipBg.height
        var windowW = root.width || (root.window ? root.window.width : 1920)
        var windowH = root.height || (root.window ? root.window.height : 1080)
        var margin = 6

        var tx
        var ty

        if (position === "bottom") {
            tx = globalPos.x + (targetW - tipW) / 2
            ty = globalPos.y + targetH + margin
            if (ty + tipH > windowH - margin) {
                ty = globalPos.y - tipH - margin
            }
        } else if (position === "top") {
            tx = globalPos.x + (targetW - tipW) / 2
            ty = globalPos.y - tipH - margin
            if (ty < margin) {
                ty = globalPos.y + targetH + margin
            }
        } else if (position === "right") {
            tx = globalPos.x + targetW + margin
            ty = globalPos.y + (targetH - tipH) / 2
            if (tx + tipW > windowW - margin) {
                tx = globalPos.x - tipW - margin
            }
        } else {
            tx = globalPos.x - tipW - margin
            ty = globalPos.y + (targetH - tipH) / 2
            if (tx < margin) {
                tx = globalPos.x + targetW + margin
            }
        }

        tx = Math.max(margin, Math.min(tx, windowW - tipW - margin))
        ty = Math.max(margin, Math.min(ty, windowH - tipH - margin))

        tooltipBg.x = tx
        tooltipBg.y = ty
    }

    Rectangle {
        id: tooltipBg
        width: tipLabel.implicitWidth + theme.sp3
        height: tipLabel.implicitHeight + theme.sp1 + 2
        radius: theme.radiusSm
        color: theme.panelStrong
        border.color: theme.borderSoft
        border.width: 1
        opacity: root.showing ? 1.0 : 0.0

        Behavior on opacity { NumberAnimation { duration: 50 } }

        Rectangle {
            anchors.fill: parent
            anchors.topMargin: 1
            anchors.bottomMargin: -1
            radius: parent.radius
            color: theme.shadowMd
            z: -1
        }

        Text {
            id: tipLabel
            anchors.centerIn: parent
            text: root.text
            color: theme.textBase
            font.family: theme.sans
            font.pixelSize: theme.fontSmall
        }
    }
}
