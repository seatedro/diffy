import QtQuick

Rectangle {
    id: root

    property bool active: false

    implicitHeight: 3
    color: "transparent"
    clip: true

    Rectangle {
        id: indicator
        height: parent.height
        width: parent.width * 0.3
        radius: 1
        color: theme.accent
        visible: root.active
        x: -width

        SequentialAnimation on x {
            running: root.active
            loops: Animation.Infinite
            NumberAnimation {
                from: -indicator.width
                to: root.width
                duration: 1200
                easing.type: Easing.InOutQuad
            }
            PauseAnimation { duration: 200 }
        }
    }
}
