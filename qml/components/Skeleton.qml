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
        width: parent.width * 0.6
        height: parent.height
        x: root.shimmerPosition * (parent.width + width) - width
        radius: parent.radius

        gradient: Gradient {
            orientation: Gradient.Horizontal
            GradientStop { position: 0.0; color: "transparent" }
            GradientStop { position: 0.4; color: Qt.rgba(theme.panel.r, theme.panel.g, theme.panel.b, 0.4) }
            GradientStop { position: 0.6; color: Qt.rgba(theme.panel.r, theme.panel.g, theme.panel.b, 0.4) }
            GradientStop { position: 1.0; color: "transparent" }
        }
    }

    NumberAnimation on shimmerPosition {
        from: 0; to: 1
        duration: 900
        loops: Animation.Infinite
        easing.type: Easing.InOutQuad
    }
}
