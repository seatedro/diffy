import QtQuick
import QtQuick.Window

Rectangle {
    id: root

    property bool horizontal: false
    property real minBefore: 160
    property real maxBefore: 400
    property real position: 240

    signal dragged(real value)

    width: horizontal ? parent.width : 5
    height: horizontal ? 5 : parent.height
    color: "transparent"

    Rectangle {
        anchors.centerIn: parent
        width: root.horizontal ? parent.width : 1
        height: root.horizontal ? 1 : parent.height
        color: dragArea.containsMouse || dragArea.pressed ? theme.accent : theme.divider

        Behavior on color {
            enabled: !(root.Window.window && root.Window.window.commandPaletteShowing)
            ColorAnimation { duration: 45 }
        }
    }

    // Wider hit area
    MouseArea {
        id: dragArea
        anchors.fill: parent
        anchors.margins: root.horizontal ? -3 : -3
        hoverEnabled: true
        cursorShape: root.horizontal ? Qt.SplitVCursor : Qt.SplitHCursor
        preventStealing: true

        property real startPos: 0
        property real startMouse: 0

        onPressed: function(mouse) {
            startPos = root.position
            startMouse = root.horizontal ? mouse.y : mouse.x
        }

        onPositionChanged: function(mouse) {
            if (!pressed) return
            var delta = (root.horizontal ? mouse.y : mouse.x) - startMouse
            var newPos = Math.max(root.minBefore, Math.min(root.maxBefore, startPos + delta))
            root.dragged(newPos)
        }
    }
}
