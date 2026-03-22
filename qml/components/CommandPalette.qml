import QtQuick
import QtQuick.Layouts
import "ColorUtils.js" as ColorUtils

Rectangle {
    id: root

    property bool showing: false
    property var items: []  // [{label, detail, category, action}]
    property var filteredItems: []
    property int selectedIdx: 0

    signal actionTriggered(var item)
    signal itemHighlighted(var item)
    signal closed()

    function open(sourceItems) {
        items = sourceItems
        searchField.text = ""
        filterItems()
        showing = true
        searchField.forceActiveFocus()
        emitHighlighted()
    }

    function close() {
        if (!showing) return
        showing = false
        searchField.text = ""
        itemHighlighted(null)
        closed()
    }

    function filterItems() {
        var query = searchField.text
        if (query.length === 0) {
            filteredItems = items
        } else {
            filteredItems = diffController.fuzzyFilter(query, items, "label")
        }
        selectedIdx = 0
        emitHighlighted()
    }

    function highlightedItem() {
        if (selectedIdx >= 0 && selectedIdx < filteredItems.length) {
            return filteredItems[selectedIdx]
        }
        return null
    }

    function emitHighlighted() {
        itemHighlighted(highlightedItem())
    }

    function activateSelected() {
        if (selectedIdx >= 0 && selectedIdx < filteredItems.length) {
            var item = filteredItems[selectedIdx]
            actionTriggered(item)
            if (!item.keepOpen) {
                close()
            }
        }
    }

    visible: showing
    anchors.fill: parent
    color: "#66000000"
    z: 250
    onSelectedIdxChanged: {
        emitHighlighted()
        if (selectedIdx >= 0 && selectedIdx < filteredItems.length) {
            resultsList.positionViewAtIndex(selectedIdx, ListView.Contain)
        }
    }

    MouseArea {
        anchors.fill: parent
        onClicked: root.close()
    }

    Rectangle {
        id: palette
        anchors.horizontalCenter: parent.horizontalCenter
        y: parent.height * 0.15
        width: Math.min(560, parent.width - theme.sp12)
        height: Math.min(paletteCol.implicitHeight, parent.height * 0.6)
        radius: theme.radiusXl
        color: theme.panel
        border.color: theme.borderSoft
        clip: true
        scale: 1.0
        opacity: root.showing ? 1.0 : 0

        Rectangle {
            anchors.fill: parent
            anchors.topMargin: 6
            anchors.bottomMargin: -6
            radius: parent.radius
            color: theme.shadowLg
            z: -1
        }

        ColumnLayout {
            id: paletteCol
            anchors.fill: parent
            spacing: 0

            // Search input
            Rectangle {
                Layout.fillWidth: true
                implicitHeight: 44
                color: "transparent"

                TextInput {
                    id: searchField
                    anchors.fill: parent
                    anchors.leftMargin: theme.sp4
                    anchors.rightMargin: theme.sp4
                    anchors.topMargin: theme.sp3
                    anchors.bottomMargin: theme.sp3
                    color: theme.textStrong
                    font.family: theme.sans
                    font.pixelSize: theme.fontSubtitle
                    clip: true
                    selectByMouse: true

                    onTextChanged: root.filterItems()

                    Keys.onUpPressed: {
                        if (root.selectedIdx > 0) root.selectedIdx--
                    }
                    Keys.onDownPressed: {
                        if (root.selectedIdx < root.filteredItems.length - 1) root.selectedIdx++
                    }
                    Keys.onReturnPressed: root.activateSelected()
                    Keys.onEscapePressed: root.close()

                    Text {
                        anchors.fill: parent
                        visible: searchField.text.length === 0
                        text: "Type to search…"
                        color: theme.textFaint
                        font.family: theme.sans
                        font.pixelSize: theme.fontSubtitle
                        verticalAlignment: Text.AlignVCenter
                    }
                }
            }

            Rectangle {
                Layout.fillWidth: true
                implicitHeight: 1
                color: theme.borderSoft
            }

            // Results
            ListView {
                id: resultsList
                Layout.fillWidth: true
                Layout.fillHeight: true
                Layout.preferredHeight: Math.min(contentHeight, 360)
                model: root.filteredItems
                clip: true
                currentIndex: root.selectedIdx
                boundsBehavior: Flickable.StopAtBounds
                highlightMoveDuration: 0
                highlightResizeDuration: 0

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
                    readonly property color selectedMetaColor: ColorUtils.withAlpha(selectedTextColor, 0.82)

                    width: ListView.view.width
                    height: 36
                    color: root.selectedIdx === index ? theme.selectionBg : (itemMouse.containsMouse ? theme.panelStrong : "transparent")

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: theme.sp4
                        anchors.rightMargin: theme.sp4
                        spacing: theme.sp2

                        Text {
                            Layout.fillWidth: true
                            text: modelData.label
                            color: root.selectedIdx === index ? selectedTextColor : theme.textBase
                            font.family: theme.sans
                            font.pixelSize: theme.fontBody
                            font.bold: root.selectedIdx === index
                            elide: Text.ElideRight
                        }

                        Text {
                            visible: (modelData.detail || "").length > 0
                            text: modelData.detail || ""
                            color: root.selectedIdx === index ? selectedMetaColor : theme.textFaint
                            font.family: theme.mono
                            font.pixelSize: theme.fontSmall
                        }

                        Text {
                            visible: (modelData.category || "").length > 0
                            text: modelData.category || ""
                            color: root.selectedIdx === index ? selectedMetaColor : theme.textFaint
                            font.family: theme.sans
                            font.pixelSize: theme.fontCaption
                        }
                    }

                    MouseArea {
                        id: itemMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onEntered: root.selectedIdx = index
                        onClicked: {
                            root.selectedIdx = index
                            root.activateSelected()
                        }
                    }
                }
            }

            // Footer hint
            Rectangle {
                Layout.fillWidth: true
                implicitHeight: 28
                color: theme.panelStrong

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: theme.sp4
                    anchors.rightMargin: theme.sp4
                    spacing: theme.sp4

                    Text {
                        text: root.filteredItems.length + " results"
                        color: theme.textFaint
                        font.family: theme.sans
                        font.pixelSize: theme.fontCaption
                    }

                    Item { Layout.fillWidth: true }

                    Text {
                        text: "↑↓ navigate  ↵ select  esc close"
                        color: theme.textFaint
                        font.family: theme.mono
                        font.pixelSize: theme.fontCaption
                    }
                }
            }
        }
    }
}
