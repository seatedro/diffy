import QtQuick

Rectangle {
    id: root

    property real shimmerPosition: 0

    implicitWidth: 200
    implicitHeight: 14
    radius: 4
    color: theme.panelStrong
    clip: true

    Rectangle {
        width: parent.width * 0.4
        height: parent.height
        x: root.shimmerPosition * (parent.width + width) - width
        color: theme.panel
        opacity: 0.5
        radius: parent.radius
    }

    NumberAnimation on shimmerPosition {
        from: 0; to: 1
        duration: 1500
        loops: Animation.Infinite
        easing.type: Easing.InOutQuad
    }
}
