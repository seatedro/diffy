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
            height: 34
            color: theme.panel

            Row {
                anchors.fill: parent
                anchors.leftMargin: 10
                anchors.rightMargin: 10
                spacing: 7

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: "Changes"
                    color: theme.textStrong
                    font.family: theme.sans
                    font.pixelSize: 10
                    font.bold: true
                }

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: repoName()
                    color: theme.textFaint
                    font.family: theme.sans
                    font.pixelSize: 8
                    elide: Text.ElideRight
                    width: 54
                }

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: files.length + " files"
                    color: theme.textFaint
                    font.family: theme.sans
                    font.pixelSize: 8
                }

                Item {
                    width: 1
                    height: 1
                }

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: "+" + totalAdditions()
                    color: theme.successText
                    font.family: theme.mono
                    font.pixelSize: 8
                }

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: "-" + totalDeletions()
                    color: theme.dangerText
                    font.family: theme.mono
                    font.pixelSize: 8
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
            anchors.topMargin: 35
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
                height: 27
                radius: 0
                color: root.selectedIndex === index ? theme.selectionBg : (mouseArea.containsMouse ? theme.panel : "transparent")
                border.width: 0

                Rectangle {
                    anchors.left: parent.left
                    anchors.top: parent.top
                    anchors.bottom: parent.bottom
                    width: root.selectedIndex === index ? 4 : 0
                    color: theme.accentStrong
                }

                Rectangle {
                    anchors.left: parent.left
                    anchors.leftMargin: 11
                    anchors.verticalCenter: parent.verticalCenter
                    width: 5
                    height: 5
                    radius: 2.5
                    color: root.statusColor(modelData.status)
                }

                Text {
                    anchors.left: parent.left
                    anchors.leftMargin: 22
                    anchors.verticalCenter: parent.verticalCenter
                    width: parent.width - counts.implicitWidth - 38
                    text: modelData.path
                    color: root.selectedIndex === index ? theme.textStrong : theme.textBase
                    font.family: theme.sans
                    font.pixelSize: 9
                    font.bold: root.selectedIndex === index
                    elide: Text.ElideMiddle
                }

                Row {
                    id: counts
                    anchors.right: parent.right
                    anchors.rightMargin: 8
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 5

                    Text {
                        visible: modelData.isBinary
                        text: "bin"
                        color: theme.textFaint
                        font.family: theme.mono
                        font.pixelSize: 7
                    }

                    Text {
                        text: "+" + modelData.additions
                        color: theme.successText
                        font.family: theme.mono
                        font.pixelSize: 7
                    }

                    Text {
                        text: "-" + modelData.deletions
                        color: theme.dangerText
                        font.family: theme.mono
                        font.pixelSize: 7
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
                font.pixelSize: 10
            }
        }
    }
}
