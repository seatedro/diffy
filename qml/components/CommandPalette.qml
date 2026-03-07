import QtQuick
import QtQuick.Layouts

Rectangle {
    id: root

    property bool showing: false
    property var items: []  // [{label, detail, category, action}]
    property var filteredItems: []
    property int selectedIdx: 0

    signal actionTriggered(var item)

    function open(sourceItems) {
        items = sourceItems
        searchField.text = ""
        filterItems()
        showing = true
        searchField.forceActiveFocus()
    }

    function close() {
        showing = false
        searchField.text = ""
    }

    function filterItems() {
        var query = searchField.text
        if (query.length === 0) {
            filteredItems = items
        } else {
            filteredItems = diffController.fuzzyFilter(query, items, "label")
        }
        selectedIdx = 0
    }

    function activateSelected() {
        if (selectedIdx >= 0 && selectedIdx < filteredItems.length) {
            actionTriggered(filteredItems[selectedIdx])
            close()
        }
    }

    visible: showing
    anchors.fill: parent
    color: "#66000000"
    z: 250

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

        // Shadow
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

                delegate: Rectangle {
                    required property int index
                    required property var modelData

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
                            color: root.selectedIdx === index ? theme.textStrong : theme.textBase
                            font.family: theme.sans
                            font.pixelSize: theme.fontBody
                            font.bold: root.selectedIdx === index
                            elide: Text.ElideRight
                        }

                        Text {
                            visible: (modelData.detail || "").length > 0
                            text: modelData.detail || ""
                            color: theme.textFaint
                            font.family: theme.mono
                            font.pixelSize: theme.fontSmall
                        }

                        Text {
                            visible: (modelData.category || "").length > 0
                            text: modelData.category || ""
                            color: theme.textFaint
                            font.family: theme.sans
                            font.pixelSize: theme.fontCaption
                        }
                    }

                    MouseArea {
                        id: itemMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
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
