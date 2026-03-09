import QtQuick
import QtQuick.Controls

Rectangle {
    id: root

    property var files: []
    property int selectedIndex: -1
    property string repoPath: ""
    property string leftRef: ""
    property string rightRef: ""
    property string compareMode: "two-dot"
    property string renderer: "builtin"
    signal fileSelected(int index)

    onSelectedIndexChanged: {
        if (selectedIndex >= 0)
            fileListView.positionViewAtIndex(selectedIndex, ListView.Contain)
    }

    function statusLabel(status) {
        if (status === "A") return "A"
        if (status === "D") return "D"
        if (status === "R") return "R"
        return "M"
    }

    function statusVariant(status) {
        if (status === "A") return "success"
        if (status === "D") return "danger"
        if (status === "R") return "accent"
        return "warning"
    }

    function totalAdditions() {
        var total = 0
        for (var i = 0; i < files.length; ++i) total += files[i].additions
        return total
    }

    function totalDeletions() {
        var total = 0
        for (var i = 0; i < files.length; ++i) total += files[i].deletions
        return total
    }

    color: "transparent"
    border.width: 0

    Rectangle {
        anchors.fill: parent
        radius: 0
        color: theme.panel
        border.width: 0

        Rectangle {
            id: sidebarHeader
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            height: 36
            color: theme.panel

            Row {
                anchors.fill: parent
                anchors.leftMargin: theme.sp3
                anchors.rightMargin: theme.sp3
                spacing: theme.sp2

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: "Changes"
                    color: theme.textStrong
                    font.family: theme.sans
                    font.pixelSize: theme.fontBody
                    font.bold: true
                }

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: files.length + (files.length === 1 ? " file" : " files")
                    color: theme.textFaint
                    font.family: theme.sans
                    font.pixelSize: theme.fontSmall
                }

                Item { width: 1; height: 1 }

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: "+" + totalAdditions()
                    color: theme.successText
                    font.family: theme.mono
                    font.pixelSize: theme.fontSmall
                }

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: "-" + totalDeletions()
                    color: theme.dangerText
                    font.family: theme.mono
                    font.pixelSize: theme.fontSmall
                }

                DiffStatsBar {
                    anchors.verticalCenter: parent.verticalCenter
                    width: 40
                    additions: totalAdditions()
                    deletions: totalDeletions()
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

        Column {
            visible: diffController.comparing
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.topMargin: 37
            z: 10
            spacing: 2

            Repeater {
                model: 6
                Rectangle {
                    width: parent.width
                    height: 30
                    color: "transparent"

                    Row {
                        anchors.verticalCenter: parent.verticalCenter
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.leftMargin: theme.sp2
                        anchors.rightMargin: theme.sp3
                        spacing: theme.sp2

                        Skeleton {
                            width: 20
                            height: 12
                            anchors.verticalCenter: parent.verticalCenter
                        }

                        Skeleton {
                            width: parent.width * (0.4 + Math.random() * 0.3)
                            height: 12
                            anchors.verticalCenter: parent.verticalCenter
                        }
                    }
                }
            }
        }

        ListView {
            id: fileListView
            anchors.fill: parent
            anchors.topMargin: 37
            model: root.files
            currentIndex: root.selectedIndex
            clip: true
            spacing: 0
            highlightFollowsCurrentItem: false
            cacheBuffer: 480
            reuseItems: true
            boundsBehavior: Flickable.StopAtBounds

            ScrollBar.vertical: ScrollBar {
                policy: fileListView.contentHeight > fileListView.height ? ScrollBar.AsNeeded : ScrollBar.AlwaysOff
                contentItem: Rectangle {
                    implicitWidth: 4
                    radius: 2
                    color: theme.textFaint
                    opacity: parent.active ? 0.6 : 0.3
                    Behavior on opacity { NumberAnimation { duration: 120 } }
                }
                background: Item {}
            }

            delegate: Rectangle {
                required property int index
                required property var modelData

                width: ListView.view.width
                height: 30
                radius: 0
                color: root.selectedIndex === index ? theme.selectionBg : (mouseArea.containsMouse ? theme.panelStrong : "transparent")
                border.width: 0
                opacity: 0

                Component.onCompleted: staggerAnim.start()

                NumberAnimation on opacity {
                    id: staggerAnim
                    running: false
                    from: 0; to: 1
                    duration: 150
                    easing.type: Easing.OutCubic
                }

                Rectangle {
                    anchors.left: parent.left
                    anchors.top: parent.top
                    anchors.bottom: parent.bottom
                    width: root.selectedIndex === index ? 3 : 0
                    color: theme.accent
                }

                Badge {
                    id: statusBadge
                    anchors.left: parent.left
                    anchors.leftMargin: theme.sp2
                    anchors.verticalCenter: parent.verticalCenter
                    text: root.statusLabel(modelData.status)
                    variant: root.statusVariant(modelData.status)
                }

                Text {
                    anchors.left: statusBadge.right
                    anchors.leftMargin: theme.sp2
                    anchors.verticalCenter: parent.verticalCenter
                    width: parent.width - statusBadge.width - counts.implicitWidth - theme.sp8 - theme.sp2
                    text: modelData.path
                    color: root.selectedIndex === index ? theme.textStrong : theme.textBase
                    font.family: theme.sans
                    font.pixelSize: theme.fontSmall + 1
                    font.bold: root.selectedIndex === index
                    elide: Text.ElideLeft
                }

                Row {
                    id: counts
                    anchors.right: parent.right
                    anchors.rightMargin: theme.sp3
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 5

                    Text {
                        visible: modelData.isBinary
                        text: "bin"
                        color: theme.textFaint
                        font.family: theme.mono
                        font.pixelSize: theme.fontSmall
                    }

                    Text {
                        text: "+" + modelData.additions
                        color: theme.successText
                        font.family: theme.mono
                        font.pixelSize: theme.fontSmall
                    }

                    Text {
                        text: "-" + modelData.deletions
                        color: theme.dangerText
                        font.family: theme.mono
                        font.pixelSize: theme.fontSmall
                    }

                    DiffStatsBar {
                        anchors.verticalCenter: parent.verticalCenter
                        width: 30
                        additions: modelData.additions
                        deletions: modelData.deletions
                    }
                }

                MouseArea {
                    id: mouseArea
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: root.fileSelected(index)
                    onContainsMouseChanged: {
                        if (containsMouse) window.showTooltip(parent, modelData.path, "right")
                        else window.hideTooltip()
                    }
                }
            }

            EmptyState {
                visible: root.files.length === 0 && !diffController.comparing
                anchors.centerIn: parent
                title: "No changes"
                subtitle: "Run compare to see files."
            }
        }
    }
}
