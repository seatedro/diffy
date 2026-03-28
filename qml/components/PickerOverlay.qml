import QtQuick
import QtQuick.Layouts
import "ColorUtils.js" as ColorUtils

Rectangle {
    id: root

    property bool showing: false
    property string title: ""
    property var model: []
    property string filterText: ""
    property string displayRole: "display"
    property string subtitleRole: ""
    property string badgeRole: ""
    property string breadcrumb: ""
    property bool showUpButton: false
    property var pinnedItem: null
    property int selectedIdx: 0

    signal itemSelected(int index, var item)
    signal dismissed()
    signal navigateUp()
    signal pinnedItemActivated()

    function filteredModel() {
        if (!model || model.length === 0) return []
        var needle = filterText.toLowerCase()
        if (needle.length === 0) return model
        var result = []
        for (var i = 0; i < model.length; ++i) {
            var item = model[i]
            var text = (typeof item === "string") ? item : (item[displayRole] || "")
            if (fuzzyMatch(text.toLowerCase(), needle)) {
                result.push(item)
            }
        }
        return result
    }

    function fuzzyMatch(text, pattern) {
        var ti = 0
        for (var pi = 0; pi < pattern.length; ++pi) {
            var found = false
            while (ti < text.length) {
                if (text[ti] === pattern[pi]) {
                    ti++
                    found = true
                    break
                }
                ti++
            }
            if (!found) return false
        }
        return true
    }

    function minIdx() {
        return pinnedItem ? -1 : 0
    }

    function maxIdx() {
        return filteredModel().length - 1
    }

    function activateSelected() {
        if (selectedIdx === -1 && pinnedItem) {
            pinnedItemActivated()
            return
        }
        var filtered = filteredModel()
        if (selectedIdx >= 0 && selectedIdx < filtered.length) {
            itemSelected(selectedIdx, filtered[selectedIdx])
        }
    }

    visible: showing
    anchors.fill: parent
    color: "#66000000"
    z: 250

    onShowingChanged: {
        if (showing) {
            searchField.text = ""
            filterText = ""
            selectedIdx = pinnedItem ? -1 : 0
            searchField.forceActiveFocus()
        }
    }

    onSelectedIdxChanged: {
        if (selectedIdx >= 0 && selectedIdx < resultList.count) {
            resultList.positionViewAtIndex(selectedIdx, ListView.Contain)
        }
    }

    MouseArea {
        anchors.fill: parent
        onClicked: root.dismissed()
    }

    Rectangle {
        id: panel
        anchors.horizontalCenter: parent.horizontalCenter
        y: parent.height * 0.15
        width: Math.min(560, parent.width - theme.sp12)
        height: Math.min(panelCol.implicitHeight, parent.height * 0.6)
        radius: theme.radiusXl
        color: theme.panel
        border.color: theme.borderSoft
        clip: true
        scale: 1.0
        opacity: root.showing ? 1.0 : 0

        Behavior on scale {
            SpringAnimation { spring: 10; damping: 0.85 }
        }
        Behavior on opacity {
            NumberAnimation { duration: 50; easing.type: Easing.OutCubic }
        }

        Rectangle {
            anchors.fill: parent
            anchors.topMargin: 6
            anchors.bottomMargin: -6
            radius: parent.radius
            color: theme.shadowLg
            z: -1
        }

        ColumnLayout {
            id: panelCol
            anchors.fill: parent
            spacing: 0

            Rectangle {
                Layout.fillWidth: true
                implicitHeight: 44
                color: "transparent"

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: root.showUpButton ? theme.sp2 : theme.sp4
                    anchors.rightMargin: theme.sp4
                    anchors.topMargin: theme.sp3
                    anchors.bottomMargin: theme.sp3
                    spacing: 0

                    Text {
                        visible: root.showUpButton
                        text: "←"
                        color: upMouse.containsMouse ? theme.textBase : theme.textFaint
                        font.family: theme.sans
                        font.pixelSize: theme.fontSubtitle
                        Layout.preferredWidth: theme.sp8
                        horizontalAlignment: Text.AlignHCenter

                        Behavior on color {
                            ColorAnimation { duration: 35 }
                        }

                        MouseArea {
                            id: upMouse
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: root.navigateUp()
                        }
                    }

                    TextInput {
                        id: searchField
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        color: theme.textStrong
                        font.family: theme.sans
                        font.pixelSize: theme.fontSubtitle
                        clip: true
                        selectByMouse: true
                        verticalAlignment: TextInput.AlignVCenter

                        onTextChanged: {
                            root.filterText = text
                            root.selectedIdx = root.pinnedItem ? -1 : 0
                        }

                        Keys.onUpPressed: {
                            if (root.selectedIdx > root.minIdx()) root.selectedIdx--
                        }
                        Keys.onDownPressed: {
                            if (root.selectedIdx < root.maxIdx()) root.selectedIdx++
                        }
                        Keys.onReturnPressed: root.activateSelected()
                        Keys.onEscapePressed: root.dismissed()

                        Text {
                            anchors.fill: parent
                            visible: searchField.text.length === 0
                            text: root.title.length > 0 ? root.title : "Type to filter\u2026"
                            color: theme.textFaint
                            font.family: theme.sans
                            font.pixelSize: theme.fontSubtitle
                            verticalAlignment: Text.AlignVCenter
                        }
                    }
                }
            }

            Rectangle {
                Layout.fillWidth: true
                implicitHeight: root.breadcrumb.length > 0 ? 22 : 0
                visible: root.breadcrumb.length > 0
                color: "transparent"

                Text {
                    anchors.fill: parent
                    anchors.leftMargin: theme.sp4
                    anchors.rightMargin: theme.sp4
                    text: root.breadcrumb
                    color: theme.textFaint
                    font.family: theme.mono
                    font.pixelSize: theme.fontCaption
                    elide: Text.ElideMiddle
                    verticalAlignment: Text.AlignVCenter
                }
            }

            Rectangle {
                Layout.fillWidth: true
                implicitHeight: 1
                color: theme.borderSoft
            }

            Rectangle {
                id: pinnedRow
                Layout.fillWidth: true
                visible: root.pinnedItem !== null
                implicitHeight: root.pinnedItem ? 36 : 0
                color: root.selectedIdx === -1 ? theme.selectionBg : (pinnedMouse.containsMouse ? theme.panelStrong : "transparent")

                readonly property color pinnedTextColor: ColorUtils.bestContrastColor(theme.selectionBg, [
                    theme.textStrong,
                    theme.textBase,
                    theme.panel,
                    theme.canvas,
                    "#101010",
                    "#f8f8f8"
                ])
                readonly property color pinnedMetaColor: ColorUtils.withAlpha(pinnedTextColor, 0.82)

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: theme.sp4
                    anchors.rightMargin: theme.sp4
                    spacing: theme.sp2

                    Text {
                        Layout.fillWidth: true
                        text: root.pinnedItem ? (root.pinnedItem.display || "") : ""
                        color: root.selectedIdx === -1 ? pinnedRow.pinnedTextColor : theme.textBase
                        font.family: theme.sans
                        font.pixelSize: theme.fontBody
                        font.bold: root.selectedIdx === -1
                        elide: Text.ElideRight
                    }

                    Badge {
                        visible: root.pinnedItem && (root.pinnedItem.badge || "").length > 0
                        text: root.pinnedItem ? (root.pinnedItem.badge || "") : ""
                        variant: "accent"
                    }
                }

                MouseArea {
                    id: pinnedMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onEntered: root.selectedIdx = -1
                    onClicked: {
                        root.selectedIdx = -1
                        root.pinnedItemActivated()
                    }
                }
            }

            ListView {
                id: resultList
                Layout.fillWidth: true
                Layout.fillHeight: true
                Layout.preferredHeight: Math.min(contentHeight, 360)
                model: root.filteredModel()
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
                    color: root.selectedIdx === index ? theme.selectionBg : (delegateMouse.containsMouse ? theme.panelStrong : "transparent")

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: theme.sp4
                        anchors.rightMargin: theme.sp4
                        spacing: theme.sp2

                        Text {
                            Layout.fillWidth: true
                            text: (typeof modelData === "string") ? modelData : (modelData[root.displayRole] || "")
                            color: root.selectedIdx === index ? selectedTextColor : theme.textBase
                            font.family: theme.sans
                            font.pixelSize: theme.fontBody
                            font.bold: root.selectedIdx === index
                            elide: Text.ElideRight
                        }

                        Text {
                            visible: root.subtitleRole.length > 0 && modelData[root.subtitleRole] !== undefined
                            text: modelData[root.subtitleRole] || ""
                            color: root.selectedIdx === index ? selectedMetaColor : theme.textFaint
                            font.family: theme.sans
                            font.pixelSize: theme.fontSmall
                        }

                        Badge {
                            visible: root.badgeRole.length > 0 && (modelData[root.badgeRole] || "").length > 0
                            text: {
                                if (root.badgeRole.length === 0) return ""
                                var val = modelData[root.badgeRole]
                                return (typeof val === "string") ? val : (val ? "\u2605" : "")
                            }
                            variant: "accent"
                        }
                    }

                    MouseArea {
                        id: delegateMouse
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
                        text: root.filteredModel().length + " results"
                        color: theme.textFaint
                        font.family: theme.sans
                        font.pixelSize: theme.fontCaption
                    }

                    Item { Layout.fillWidth: true }

                    Text {
                        text: "\u2191\u2193 navigate  \u21b5 select  esc close"
                        color: theme.textFaint
                        font.family: theme.mono
                        font.pixelSize: theme.fontCaption
                    }
                }
            }
        }
    }
}
