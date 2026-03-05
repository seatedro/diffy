import QtQuick

Rectangle {
    id: root

    required property QtObject theme
    property var files: []
    property int selectedIndex: -1
    property string repoPath: ""
    property string leftRef: ""
    property string rightRef: ""
    property string compareMode: "two-dot"
    property string renderer: "builtin"
    signal fileSelected(int index)

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

    function repoName() {
        if (repoPath.length === 0) {
            return "No repository"
        }
        var parts = repoPath.split("/")
        return parts.length > 0 ? parts[parts.length - 1] : repoPath
    }

    function compareLabel() {
        if (compareMode === "three-dot") {
            return leftRef + "..." + rightRef
        }
        if (compareMode === "single-commit") {
            return rightRef.length > 0 ? rightRef : leftRef
        }
        return leftRef + ".." + rightRef
    }

    function totalAdditions() {
        var total = 0
        for (var i = 0; i < files.length; ++i) {
            total += files[i].additions
        }
        return total
    }

    function totalDeletions() {
        var total = 0
        for (var i = 0; i < files.length; ++i) {
            total += files[i].deletions
        }
        return total
    }

    color: "transparent"
    border.width: 0

    Rectangle {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        anchors.margins: 0
        radius: 0
        color: theme.panel
        border.width: 0

        Rectangle {
            id: sidebarHeader
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            height: 56
            color: theme.panelStrong

            Column {
                anchors.fill: parent
                anchors.leftMargin: 12
                anchors.rightMargin: 10
                anchors.topMargin: 8
                anchors.bottomMargin: 8
                spacing: 4

                Row {
                    spacing: 8

                    Text {
                        text: "Changes"
                        color: theme.textStrong
                        font.family: theme.sans
                        font.pixelSize: 12
                        font.bold: true
                    }

                    Text {
                        text: files.length + " files"
                        color: theme.textFaint
                        font.family: theme.sans
                        font.pixelSize: 10
                    }
                }

                Row {
                    spacing: 10

                    Text {
                        text: "+" + totalAdditions()
                        color: theme.successText
                        font.family: theme.mono
                        font.pixelSize: 10
                    }

                    Text {
                        text: "-" + totalDeletions()
                        color: theme.dangerText
                        font.family: theme.mono
                        font.pixelSize: 10
                    }

                    Text {
                        text: compareLabel()
                        color: theme.textFaint
                        font.family: theme.mono
                        font.pixelSize: 9
                        elide: Text.ElideRight
                        width: parent.width - 110
                    }
                }
            }
        }

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: sidebarHeader.bottom
            height: 1
            color: theme.divider
        }

        ListView {
            anchors.fill: parent
            anchors.topMargin: 57
            anchors.leftMargin: 0
            anchors.rightMargin: 0
            anchors.bottomMargin: 0
            model: root.files
            clip: true
            spacing: 0
            cacheBuffer: 480
            reuseItems: true
            boundsBehavior: Flickable.StopAtBounds

            delegate: Rectangle {
                required property int index
                required property var modelData

                width: ListView.view.width
                height: 34
                radius: 0
                color: root.selectedIndex === index ? theme.selectionBg : (mouseArea.containsMouse ? theme.panelStrong : "transparent")
                border.width: 0

                Rectangle {
                    anchors.left: parent.left
                    anchors.top: parent.top
                    anchors.bottom: parent.bottom
                    width: root.selectedIndex === index ? 2 : 0
                    color: theme.selectionBorder
                }

                Rectangle {
                    anchors.left: parent.left
                    anchors.leftMargin: 10
                    anchors.verticalCenter: parent.verticalCenter
                    width: 6
                    height: 6
                    radius: 3
                    color: root.statusColor(modelData.status)
                }

                Text {
                    anchors.left: parent.left
                    anchors.leftMargin: 24
                    anchors.verticalCenter: parent.verticalCenter
                    width: parent.width - counts.implicitWidth - 44
                    text: modelData.path
                    color: root.selectedIndex === index ? theme.textStrong : theme.textBase
                    font.family: theme.sans
                    font.pixelSize: 11
                    elide: Text.ElideMiddle
                }

                Row {
                    id: counts
                    anchors.right: parent.right
                    anchors.rightMargin: 10
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 8

                    Text {
                        visible: modelData.isBinary
                        text: "bin"
                        color: theme.warningText
                        font.family: theme.mono
                        font.pixelSize: 9
                    }

                    Text {
                        text: "+" + modelData.additions
                        color: theme.successText
                        font.family: theme.mono
                        font.pixelSize: 9
                    }

                    Text {
                        text: "-" + modelData.deletions
                        color: theme.dangerText
                        font.family: theme.mono
                        font.pixelSize: 9
                    }
                }

                MouseArea {
                    id: mouseArea
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: root.fileSelected(index)
                }
            }

            Text {
                visible: root.files.length === 0
                anchors.centerIn: parent
                text: "Run compare to populate the changes list."
                color: theme.textFaint
                font.family: theme.sans
                font.pixelSize: 11
            }
        }
    }
}
