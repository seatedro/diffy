import QtQuick

Rectangle {
    id: root

    property int elevation: 1
    property bool hoverLift: false
    property bool hovering: hoverArea.containsMouse

    color: theme.panel
    radius: theme.radiusLg
    border.color: theme.borderSoft
    border.width: 1

    // Shadow layer
    Rectangle {
        anchors.fill: parent
        anchors.topMargin: elevation === 1 ? 1 : (elevation === 2 ? 2 : 4)
        anchors.leftMargin: 0
        anchors.rightMargin: 0
        anchors.bottomMargin: elevation === 1 ? -1 : (elevation === 2 ? -2 : -4)
        radius: root.radius
        color: elevation === 1 ? theme.shadowSm : (elevation === 2 ? theme.shadowMd : theme.shadowLg)
        z: -1
    }

    // Hover lift
    transform: Translate {
        y: root.hoverLift && root.hovering ? -1 : 0
        Behavior on y { NumberAnimation { duration: 120; easing.type: Easing.OutCubic } }
    }

    Behavior on border.color {
        ColorAnimation { duration: 90 }
    }

    MouseArea {
        id: hoverArea
        anchors.fill: parent
        hoverEnabled: root.hoverLift
        acceptedButtons: Qt.NoButton
        propagateComposedEvents: true
    }
}
