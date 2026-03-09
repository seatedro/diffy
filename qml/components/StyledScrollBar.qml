import QtQuick
import QtQuick.Controls

ScrollBar {
    id: root
    property real barWidth: 5
    property bool isVertical: true

    contentItem: Rectangle {
        implicitWidth: root.isVertical ? root.barWidth : 0
        implicitHeight: root.isVertical ? 0 : root.barWidth
        radius: root.barWidth / 2
        color: theme.textFaint
        opacity: root.active ? 0.5 : 0.15
        Behavior on opacity { NumberAnimation { duration: 45 } }
    }
    background: Item {}
}
