import QtQuick
import QtQuick.Controls
import "ColorUtils.js" as ColorUtils

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

    function filteredFiles() {
        if (filterInput.text.length === 0) return files
        var needle = filterInput.text.toLowerCase()
        var result = []
        for (var i = 0; i < files.length; ++i) {
            if (files[i].path.toLowerCase().indexOf(needle) >= 0)
                result.push(files[i])
        }
        return result
    }

    function originalIndex(filteredIdx) {
        if (filterInput.text.length === 0) return filteredIdx
        var filtered = filteredFiles()
        if (filteredIdx < 0 || filteredIdx >= filtered.length) return -1
        var item = filtered[filteredIdx]
        for (var i = 0; i < files.length; ++i) {
            if (files[i].path === item.path) return i
        }
        return -1
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
            id: headerDivider
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: sidebarHeader.bottom
            height: 1
            color: theme.divider
        }

        Rectangle {
            id: filterBar
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: headerDivider.bottom
            height: filterInput.activeFocus || filterInput.text.length > 0 ? 28 : 0
            color: theme.panel
            clip: true
            visible: height > 0

            Behavior on height { NumberAnimation { duration: 50 } }

            InputField {
                id: filterInput
                anchors.fill: parent
                anchors.margins: 2
                compact: true
                monospace: true
                placeholderText: "Filter files\u2026"
                borderless: false
            }
        }

        Column {
            visible: diffController.comparing
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: filterBar.visible ? filterBar.bottom : headerDivider.bottom
            anchors.topMargin: 1
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
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: filterBar.visible ? filterBar.bottom : headerDivider.bottom
            anchors.topMargin: 1
            anchors.bottom: parent.bottom
            model: root.filteredFiles()
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
                    Behavior on opacity { NumberAnimation { duration: 45 } }
                }
                background: Item {}
            }

            delegate: Rectangle {
                required property int index
                required property var modelData

                readonly property color selectedTextColor: ColorUtils.bestContrastColor(theme.selectionBg, [
                    theme.textStrong,
                    theme.textBase,
                    theme.panel,
                    theme.canvas,
                    "#101010",
                    "#f8f8f8"
                ])

                width: ListView.view.width
                height: 30
                radius: 0
                color: root.selectedIndex === root.originalIndex(index) ? theme.selectionBg : (mouseArea.containsMouse ? theme.panelStrong : "transparent")
                border.width: 0
                opacity: 0

                Component.onCompleted: staggerDelay.start()

                Timer {
                    id: staggerDelay
                    interval: Math.min(index * 15, 200)
                    repeat: false
                    onTriggered: staggerAnim.start()
                }

                NumberAnimation on opacity {
                    id: staggerAnim
                    running: false
                    from: 0; to: 1
                    duration: 60
                    easing.type: Easing.OutCubic
                }

                Rectangle {
                    anchors.left: parent.left
                    anchors.top: parent.top
                    anchors.bottom: parent.bottom
                    width: root.selectedIndex === root.originalIndex(index) ? 3 : 0
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
                    color: root.selectedIndex === root.originalIndex(index) ? selectedTextColor : theme.textBase
                    font.family: theme.sans
                    font.pixelSize: theme.fontSmall + 1
                    font.bold: root.selectedIndex === root.originalIndex(index)
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
                    onClicked: root.fileSelected(root.originalIndex(index))
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

    Shortcut {
        sequence: "/"
        onActivated: filterInput.forceActiveFocus()
    }
}
