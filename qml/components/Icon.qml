import QtQuick

Item {
    id: root

    property string svg: ""
    property color color: theme.textFaint
    property real size: 16

    function iconGlyph() {
        if (root.svg.indexOf('m21 21-4.3-4.3') !== -1)
            return "⌕"
        if (root.svg.indexOf('M15 6a9 9 0 0 0-9 9V3') !== -1)
            return "⎇"
        if (root.svg.indexOf('m16 3 4 4-4 4') !== -1)
            return "⇄"
        if (root.svg.indexOf('m12 19-7-7 7-7') !== -1)
            return "←"
        if (root.svg.indexOf('m9 18 6-6-6-6') !== -1)
            return "›"
        if (root.svg.indexOf('m6 9 6 6 6-6') !== -1)
            return "⌄"
        if (root.svg.indexOf('M15 6v12a3 3') !== -1)
            return "⌘"
        return "•"
    }

    implicitWidth: size
    implicitHeight: size

    Text {
        anchors.centerIn: parent
        text: root.iconGlyph()
        color: root.color
        font.family: theme.sans
        font.pixelSize: root.size
        font.bold: text === "⌘" || text === "⎇"
        renderType: Text.NativeRendering
    }
}
