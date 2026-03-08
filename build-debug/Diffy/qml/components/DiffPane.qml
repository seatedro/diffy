import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Diffy.Native 1.0

Rectangle {
    id: root

    property var fileData: ({})
    property var rowsModel: []
    property string layoutMode: "unified"
    property string leftRef: ""
    property string rightRef: ""
    property string renderer: "builtin"
    property bool wrapEnabled: false
    property int wrapColumn: 0
    signal nextFileRequested()
    signal previousFileRequested()

    function focusSurface() {
        surface.forceActiveFocus()
    }
    property var surfacePalette: ({
        "canvas": theme.canvas,
        "panelTint": theme.panelTint,
        "panelStrong": theme.panelStrong,
        "divider": theme.divider,
        "textStrong": theme.textStrong,
        "textBase": theme.textBase,
        "textMuted": theme.textMuted,
        "textFaint": theme.textFaint,
        "selectionBg": theme.selectionBg,
        "accentSoft": theme.accentSoft,
        "successText": theme.successText,
        "successBorder": theme.successBorder,
        "dangerText": theme.dangerText,
        "dangerBorder": theme.dangerBorder,
        "lineContext": theme.lineContext,
        "lineContextAlt": theme.lineContextAlt,
        "lineAdd": theme.lineAdd,
        "lineAddAccent": theme.lineAddAccent,
        "lineDel": theme.lineDel,
        "lineDelAccent": theme.lineDelAccent
    })

    function hasData() {
        return fileData && fileData.path !== undefined
    }

    function statusColor(status) {
        if (status === "A") return theme.successText
        if (status === "D") return theme.dangerText
        if (status === "R") return theme.accentStrong
        return theme.warningText
    }

    function statusFill(status) {
        if (status === "A") return theme.successBg
        if (status === "D") return theme.dangerBg
        if (status === "R") return theme.accentSoft
        return theme.warningBg
    }

    function statusLabel(status) {
        if (status === "A") return "Added"
        if (status === "D") return "Deleted"
        if (status === "R") return "Renamed"
        return "Modified"
    }

    color: "transparent"
    border.width: 0

    Rectangle {
        id: headerPanel
        visible: !root.hasData() || fileData.isBinary
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        radius: 0
        color: theme.canvas
        border.width: 0
        implicitHeight: !root.hasData() ? 76 : 34

        RowLayout {
            visible: root.hasData() && fileData.isBinary
            x: 4
            y: 4
            width: parent.width - 8
            spacing: 8

            Rectangle {
                Layout.preferredWidth: statusText.implicitWidth + 16
                width: statusText.implicitWidth + 16
                height: 14
                radius: 4
                color: root.hasData() ? root.statusFill(fileData.status) : theme.panelStrong

                Text {
                    id: statusText
                    anchors.centerIn: parent
                    text: root.hasData() ? root.statusLabel(fileData.status) : ""
                    color: root.hasData() ? root.statusColor(fileData.status) : theme.textMuted
                    font.family: theme.sans
                    font.pixelSize: 7
                    font.bold: true
                }
            }

            Text {
                Layout.fillWidth: true
                text: root.hasData() ? fileData.path : ""
                color: theme.textStrong
                font.family: theme.sans
                font.pixelSize: 11
                font.bold: true
                elide: Text.ElideMiddle
            }

            RowLayout {
                id: metadata
                Layout.alignment: Qt.AlignRight
                spacing: 7

                Text {
                    text: "+" + fileData.additions
                    color: theme.successText
                    font.family: theme.mono
                    font.pixelSize: 8
                }

                Text {
                    text: "-" + fileData.deletions
                    color: theme.dangerText
                    font.family: theme.mono
                    font.pixelSize: 8
                }
            }
        }

        EmptyState {
            visible: !root.hasData()
            anchors.centerIn: parent
            icon: "◇"
            title: "No diff selected"
            subtitle: "Choose refs and run compare, then select a file."
        }

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 1
            color: theme.divider
        }
    }

    Rectangle {
        visible: root.hasData() && fileData.isBinary
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: headerPanel.visible ? headerPanel.bottom : parent.top
        anchors.bottom: parent.bottom
        radius: 0
        color: theme.canvas
        border.color: theme.warningBorder

        EmptyState {
            anchors.centerIn: parent
            icon: "⬡"
            title: "Binary or non-text change"
            subtitle: "This file only exposes metadata in the current renderer."
        }
    }

    Rectangle {
        visible: root.hasData() && !fileData.isBinary
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: headerPanel.visible ? headerPanel.bottom : parent.top
        anchors.bottom: parent.bottom
        radius: 0
        color: theme.canvas
        border.width: 0

        Flickable {
            id: diffViewport
            objectName: "diffViewport"
            anchors.fill: parent
            clip: true
            contentWidth: Math.max(width, surface.contentWidth)
            contentHeight: surface.contentHeight
            boundsBehavior: Flickable.StopAtBounds

            ScrollBar.vertical: ScrollBar {
                policy: diffViewport.contentHeight > diffViewport.height ? ScrollBar.AlwaysOn : ScrollBar.AlwaysOff
                contentItem: Rectangle {
                    implicitWidth: 6
                    radius: 3
                    color: theme.textFaint
                    opacity: parent.active ? 0.6 : 0.3
                    Behavior on opacity { NumberAnimation { duration: 120 } }
                }
                background: Item {}
            }

            ScrollBar.horizontal: ScrollBar {
                id: hScrollBar
                policy: root.layoutMode === "split" ? ScrollBar.AlwaysOff : (diffViewport.contentWidth > diffViewport.width ? ScrollBar.AlwaysOn : ScrollBar.AlwaysOff)
                contentItem: Rectangle {
                    implicitHeight: 6
                    radius: 3
                    color: theme.textFaint
                    opacity: hScrollBar.active ? 0.6 : 0.3
                    Behavior on opacity { NumberAnimation { duration: 120 } }
                }
                background: Item {}
            }

            DiffSurface {
                id: surface
                objectName: "diffSurface"
                x: diffViewport.contentX
                y: diffViewport.contentY
                width: diffViewport.width
                height: diffViewport.height
                focus: true
                activeFocusOnTab: true
                rowsModel: root.rowsModel
                layoutMode: root.layoutMode
                filePath: root.hasData() ? fileData.path : ""
                fileStatus: root.hasData() ? fileData.status : ""
                additions: root.hasData() ? fileData.additions : 0
                deletions: root.hasData() ? fileData.deletions : 0
                viewportX: diffViewport.contentX
                viewportY: diffViewport.contentY
                viewportHeight: diffViewport.height
                palette: root.surfacePalette
                wrapEnabled: root.wrapEnabled
                wrapColumn: root.wrapColumn
                monoFontFamily: theme.mono
                onScrollToYRequested: function(value) {
                    var maxScroll = Math.max(0, diffViewport.contentHeight - diffViewport.height)
                    diffViewport.contentY = Math.max(0, Math.min(value, maxScroll))
                }
                onNextFileRequested: root.nextFileRequested()
                onPreviousFileRequested: root.previousFileRequested()
            }
        }
    }
}
