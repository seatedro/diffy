import QtQuick
import QtQuick.Layouts
import Diffy.Native 1.0

Rectangle {
    id: root

    required property QtObject theme
    property var fileData: ({})
    property var rowsModel: []
    property string layoutMode: "unified"
    property string leftRef: ""
    property string rightRef: ""
    property string renderer: "builtin"
    property var surfacePalette: ({
        "canvas": theme.canvas,
        "panelTint": theme.panelTint,
        "panelStrong": theme.panelStrong,
        "divider": theme.divider,
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
        "lineAdd": theme.lineAdd,
        "lineAddAccent": theme.lineAddAccent,
        "lineDel": theme.lineDel,
        "lineDelAccent": theme.lineDelAccent
    })

    function hasData() {
        return fileData && fileData.path !== undefined
    }

    function statusColor(status) {
        if (status === "A") {
            return theme.successText
        }
        if (status === "D") {
            return theme.dangerText
        }
        if (status === "R") {
            return theme.accentStrong
        }
        return theme.warningText
    }

    function statusFill(status) {
        if (status === "A") {
            return theme.successBg
        }
        if (status === "D") {
            return theme.dangerBg
        }
        if (status === "R") {
            return theme.accentSoft
        }
        return theme.warningBg
    }

    function statusLabel(status) {
        if (status === "A") {
            return "Added"
        }
        if (status === "D") {
            return "Deleted"
        }
        if (status === "R") {
            return "Renamed"
        }
        return "Modified"
    }

    color: "transparent"
    border.width: 0

    Rectangle {
        id: headerPanel
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: 0
        radius: 0
        color: theme.panelStrong
        border.width: 0
        implicitHeight: root.hasData() ? 42 : 76

        Column {
            anchors.fill: parent
            anchors.margins: 6
            spacing: 3

            RowLayout {
                visible: root.hasData()
                width: parent.width
                spacing: 8

                Rectangle {
                    Layout.preferredWidth: statusText.implicitWidth + 16
                    width: statusText.implicitWidth + 16
                    height: 15
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

                Rectangle {
                    Layout.preferredWidth: rendererText.implicitWidth + 16
                    width: rendererText.implicitWidth + 16
                    height: 15
                    radius: 4
                    color: renderer === "difftastic" ? theme.warningBg : theme.accentSoft
                    border.color: renderer === "difftastic" ? theme.warningBorder : theme.borderSoft

                    Text {
                        id: rendererText
                        anchors.centerIn: parent
                        text: renderer === "difftastic" ? "difftastic" : "built-in"
                        color: renderer === "difftastic" ? theme.warningText : theme.accentStrong
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
                    font.pixelSize: 12
                    font.bold: true
                    elide: Text.ElideMiddle
                }

                RowLayout {
                    id: metadata
                    Layout.alignment: Qt.AlignRight
                    spacing: 8

                    Text {
                        text: leftRef + " -> " + rightRef
                        color: theme.textMuted
                        font.family: theme.mono
                        font.pixelSize: 8
                    }

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

                    Text {
                        text: layoutMode === "split" ? "split" : "unified"
                        color: theme.textFaint
                        font.family: theme.sans
                        font.pixelSize: 8
                    }
                }
            }

            Column {
                visible: !root.hasData()
                anchors.centerIn: parent
                width: parent.width
                spacing: 6

                Text {
                    width: parent.width
                    text: "No diff selected"
                    color: theme.textStrong
                    font.family: theme.sans
                    font.pixelSize: 16
                    font.bold: true
                    horizontalAlignment: Text.AlignHCenter
                }

                Text {
                    width: parent.width
                    text: "Open a repository, choose refs, run compare, then select a file."
                    color: theme.textMuted
                    font.family: theme.sans
                    font.pixelSize: 11
                    horizontalAlignment: Text.AlignHCenter
                }
            }
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
        anchors.top: headerPanel.bottom
        anchors.bottom: parent.bottom
        anchors.margins: 0
        radius: 0
        color: theme.canvas
        border.color: theme.warningBorder

        Column {
            anchors.centerIn: parent
            spacing: 6

            Text {
                anchors.horizontalCenter: parent.horizontalCenter
                text: "Binary or non-text change"
                color: theme.textStrong
                font.family: theme.sans
                font.pixelSize: 20
                font.bold: true
            }

            Text {
                anchors.horizontalCenter: parent.horizontalCenter
                text: "This file only exposes metadata in the current renderer."
                color: theme.warningText
                font.family: theme.sans
                font.pixelSize: 13
            }
        }
    }

    Rectangle {
        visible: root.hasData() && !fileData.isBinary
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: headerPanel.bottom
        anchors.bottom: parent.bottom
        anchors.margins: 0
        radius: 0
        color: theme.canvas
        border.width: 0

        Flickable {
            id: diffViewport
            anchors.fill: parent
            anchors.margins: 0
            clip: true
            contentWidth: Math.max(width, surface.contentWidth)
            contentHeight: surface.contentHeight
            boundsBehavior: Flickable.StopAtBounds

            DiffSurface {
                id: surface
                objectName: "diffSurface"
                width: diffViewport.width
                height: diffViewport.height
                rowsModel: root.rowsModel
                layoutMode: root.layoutMode
                viewportX: diffViewport.contentX
                viewportY: diffViewport.contentY
                viewportHeight: diffViewport.height
                palette: root.surfacePalette
                monoFontFamily: theme.mono
            }
        }
    }
}
