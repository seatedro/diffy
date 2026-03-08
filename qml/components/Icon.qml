import QtQuick

Item {
    id: root

    property string svg: ""
    property color color: theme.textFaint
    property real size: 16

    implicitWidth: size
    implicitHeight: size

    Image {
        anchors.fill: parent
        sourceSize: Qt.size(root.size * 2, root.size * 2)
        source: root.svg.length > 0
            ? "data:image/svg+xml," + encodeURIComponent(root.svg.replace('stroke="currentColor"', 'stroke="' + root.color + '"'))
            : ""
        smooth: true
    }
}
