import QtQuick
import QtQuick.Window

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
        Behavior on y {
            enabled: !(root.Window.window && root.Window.window.commandPaletteShowing)
            NumberAnimation { duration: 50; easing.type: Easing.OutCubic }
        }
    }

    Behavior on border.color {
        enabled: !(root.Window.window && root.Window.window.commandPaletteShowing)
        ColorAnimation { duration: 35 }
    }

    MouseArea {
        id: hoverArea
        anchors.fill: parent
        hoverEnabled: root.hoverLift
        acceptedButtons: Qt.NoButton
        propagateComposedEvents: true
    }

    Rectangle {
        anchors.fill: parent
        anchors.margins: -2
        radius: root.radius + 2
        color: "transparent"
        border.width: 2
        border.color: theme.accent
        visible: root.activeFocus
    }
}
