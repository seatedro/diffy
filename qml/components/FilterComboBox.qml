import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Item {
    id: root

    property bool open: false
    property Item anchorItem: null
    property string placeholder: "Search\u2026"
    property var model: []
    property int selectedIndex: -1
    property string footerText: ""

    signal itemSelected(var item, int index)

    function show(anchor) {
        anchorItem = anchor
        open = true
        searchField.text = ""
        searchField.forceActiveFocus()
    }

    function hide() {
        open = false
        searchField.text = ""
    }

    function moveSelection(delta) {
        if (model.length === 0) return
        var idx = selectedIndex
        for (var step = 0; step < model.length; ++step) {
            idx += delta
            if (idx < 0) idx = model.length - 1
            if (idx >= model.length) idx = 0
            if (!model[idx].isHeader) {
                selectedIndex = idx
                listView.positionViewAtIndex(idx, ListView.Contain)
                return
            }
        }
    }

    function activateSelected() {
        if (selectedIndex >= 0 && selectedIndex < model.length && !model[selectedIndex].isHeader) {
            itemSelected(model[selectedIndex], selectedIndex)
            hide()
        }
    }

    readonly property real searchHeight: 32
    readonly property real sepHeight: 1
    readonly property real footerHeight: 20
    property real clampedListHeight: 0
    readonly property real panelHeight: searchHeight + sepHeight + clampedListHeight + footerHeight

    onModelChanged: recomputeListHeight()

    function recomputeListHeight() {
        var h = 0
        for (var i = 0; i < model.length; ++i)
            h += (model[i].isHeader ? 20 : 26)
        clampedListHeight = Math.max(0, Math.min(h, 260))
    }

    property alias searchText: searchField.text

    visible: open
    anchors.fill: parent
    z: 300

    // Dismiss backdrop
    MouseArea {
        anchors.fill: parent
        onClicked: root.hide()
    }

    Rectangle {
        id: panel

        x: {
            if (!root.anchorItem) return (root.width - width) / 2
            var pt = root.anchorItem.mapToItem(root, 0, 0)
            return pt.x
        }
        y: {
            if (!root.anchorItem) return root.height * 0.15
            var pt = root.anchorItem.mapToItem(root, 0, root.anchorItem.height)
            return pt.y - 1
        }

        width: root.anchorItem ? root.anchorItem.width : Math.min(380, root.width - 32)
        height: root.panelHeight

        radius: 0
        color: theme.panel
        border.color: theme.borderSoft
        border.width: 1
        clip: true

        // Bottom shadow
        Rectangle {
            x: 4; y: parent.height - 1
            width: parent.width - 8; height: 5
            opacity: 0.12
            gradient: Gradient {
                GradientStop { position: 0.0; color: "#000000" }
                GradientStop { position: 1.0; color: "transparent" }
            }
        }

        // Search
        Item {
            id: searchRow
            x: 0; y: 0
            width: parent.width; height: root.searchHeight

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: theme.sp3
                anchors.rightMargin: theme.sp3
                spacing: theme.sp2

                Icon {
                    svg: '<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/></svg>'
                    size: 12
                    color: theme.textFaint
                }

                TextInput {
                    id: searchField
                    Layout.fillWidth: true
                    color: theme.textStrong
                    font.family: theme.mono
                    font.pixelSize: theme.fontSmall + 1
                    clip: true
                    selectByMouse: true

                    Keys.onUpPressed: root.moveSelection(-1)
                    Keys.onDownPressed: root.moveSelection(1)
                    Keys.onReturnPressed: root.activateSelected()
                    Keys.onEscapePressed: root.hide()

                    Text {
                        anchors.fill: parent
                        visible: searchField.text.length === 0
                        text: root.placeholder
                        color: theme.textFaint
                        font: searchField.font
                        verticalAlignment: Text.AlignVCenter
                    }
                }
            }
        }

        // Separator
        Rectangle {
            x: 0; y: root.searchHeight
            width: parent.width; height: root.sepHeight
            color: theme.borderSoft
        }

        // List
        ListView {
            id: listView
            x: 0; y: root.searchHeight + root.sepHeight
            width: parent.width
            height: root.clampedListHeight
            model: root.model
            clip: true
            currentIndex: root.selectedIndex
            boundsBehavior: Flickable.StopAtBounds

            ScrollBar.vertical: ScrollBar {
                policy: listView.contentHeight > listView.height ? ScrollBar.AsNeeded : ScrollBar.AlwaysOff
                contentItem: Rectangle {
                    implicitWidth: 3; radius: 1.5
                    color: theme.textFaint
                    opacity: parent.active ? 0.5 : 0.15
                }
                background: Item {}
            }

            delegate: Item {
                required property int index
                required property var modelData

                width: ListView.view.width
                height: modelData.isHeader ? 20 : 26

                // Section header
                Text {
                    visible: modelData.isHeader === true
                    anchors.left: parent.left
                    anchors.leftMargin: theme.sp3
                    anchors.verticalCenter: parent.verticalCenter
                    text: modelData.label || ""
                    color: theme.textFaint
                    font.family: theme.sans
                    font.pixelSize: 8
                    font.bold: true
                    font.letterSpacing: 0.5
                }

                // Selectable row
                Rectangle {
                    anchors.fill: parent
                    anchors.leftMargin: 3; anchors.rightMargin: 3
                    visible: !modelData.isHeader
                    radius: theme.radiusSm
                    color: root.selectedIndex === index
                        ? theme.selectionBg
                        : (rowMouse.containsMouse ? theme.panelStrong : "transparent")

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: theme.sp2
                        anchors.rightMargin: theme.sp2
                        spacing: theme.sp2

                        Text {
                            Layout.fillWidth: true
                            text: modelData.label || ""
                            color: root.selectedIndex === index ? theme.textStrong : theme.textBase
                            font.family: theme.mono
                            font.pixelSize: theme.fontSmall
                            font.bold: root.selectedIndex === index
                            elide: Text.ElideRight
                        }

                        Badge {
                            visible: modelData.badge === "HEAD"
                            text: "HEAD"
                            variant: "accent"
                        }

                        Text {
                            visible: (modelData.detail || "").length > 0
                            text: modelData.detail || ""
                            color: theme.textFaint
                            font.family: theme.sans
                            font.pixelSize: 8
                        }
                    }

                    MouseArea {
                        id: rowMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            root.selectedIndex = index
                            root.activateSelected()
                        }
                    }
                }
            }
        }

        // Footer
        Rectangle {
            x: 0; y: root.searchHeight + root.sepHeight + root.clampedListHeight
            width: parent.width; height: root.footerHeight
            color: theme.panelStrong

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: theme.sp3
                anchors.rightMargin: theme.sp3

                Text {
                    text: root.footerText
                    color: theme.textFaint
                    font.family: theme.sans
                    font.pixelSize: 8
                }

                Item { Layout.fillWidth: true }

                Text {
                    text: "\u2191\u2193  \u21B5  esc"
                    color: theme.textFaint
                    font.family: theme.mono
                    font.pixelSize: 8
                }
            }
        }
    }
}
