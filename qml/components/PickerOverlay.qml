import QtQuick
import QtQuick.Layouts

Item {
    id: root

    property bool showing: false
    property string title: ""
    property var model: []
    property string filterText: ""
    property string displayRole: "display"
    property string subtitleRole: ""
    property string badgeRole: ""

    signal itemSelected(int index, var item)
    signal dismissed()

    visible: showing

    onShowingChanged: {
        if (showing) {
            searchField.text = ""
            filterText = ""
            resultList.currentIndex = 0
            searchField.forceActiveFocus()
        }
    }

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

    Rectangle {
        anchors.fill: parent
        color: "#80000000"
        opacity: root.showing ? 1.0 : 0.0
        Behavior on opacity { NumberAnimation { duration: 50 } }
    }

    MouseArea {
        anchors.fill: parent
        onClicked: function(mouse) {
            if (!panel.contains(Qt.point(mouse.x - panel.x, mouse.y - panel.y))) {
                root.dismissed()
            }
        }
    }

    Rectangle {
        id: panel
        anchors.centerIn: parent
        width: Math.min(parent.width - 48, 480)
        height: Math.min(parent.height - 48, 420)
        radius: 10
        color: theme.toolbarBg
        border.color: theme.borderStrong

        focus: root.showing

        Keys.onPressed: function(event) {
            if (event.key === Qt.Key_Escape) {
                root.dismissed()
                event.accepted = true
                return
            }
            if (event.key === Qt.Key_Up) {
                resultList.decrementCurrentIndex()
                event.accepted = true
                return
            }
            if (event.key === Qt.Key_Down) {
                resultList.incrementCurrentIndex()
                event.accepted = true
                return
            }
            if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                var filtered = root.filteredModel()
                if (resultList.currentIndex >= 0 && resultList.currentIndex < filtered.length) {
                    root.itemSelected(resultList.currentIndex, filtered[resultList.currentIndex])
                }
                event.accepted = true
            }
        }

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: 10
            spacing: 6

            Text {
                visible: root.title.length > 0
                text: root.title
                color: theme.textStrong
                font.family: theme.sans
                font.pixelSize: 12
                font.bold: true
            }

            Rectangle {
                Layout.fillWidth: true
                implicitHeight: 30
                radius: 5
                color: theme.panelStrong
                border.color: searchField.activeFocus ? theme.selectionBorder : theme.borderSoft

                TextInput {
                    id: searchField
                    anchors.fill: parent
                    anchors.leftMargin: 10
                    anchors.rightMargin: 10
                    anchors.topMargin: 6
                    anchors.bottomMargin: 6
                    color: theme.textStrong
                    font.family: theme.mono
                    font.pixelSize: 11
                    clip: true
                    selectByMouse: true
                    onTextChanged: {
                        root.filterText = text
                        resultList.currentIndex = 0
                    }
                }

                Text {
                    anchors.fill: searchField
                    color: theme.textFaint
                    font.family: theme.mono
                    font.pixelSize: 11
                    verticalAlignment: Text.AlignVCenter
                    visible: searchField.text.length === 0 && !searchField.activeFocus
                    text: "Type to filter…"
                }
            }

            ListView {
                id: resultList
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                spacing: 0
                model: root.filteredModel()
                currentIndex: 0

                delegate: Rectangle {
                    required property int index
                    required property var modelData

                    width: ListView.view.width
                    height: 28
                    radius: 4
                    color: resultList.currentIndex === index ? theme.panelStrong : (delegateMouse.containsMouse ? theme.panel : "transparent")

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 8
                        anchors.rightMargin: 8
                        spacing: 8

                        Text {
                            text: (typeof modelData === "string") ? modelData : (modelData[root.displayRole] || "")
                            color: resultList.currentIndex === index ? theme.textStrong : theme.textBase
                            font.family: theme.mono
                            font.pixelSize: 11
                            Layout.fillWidth: true
                            elide: Text.ElideRight
                        }

                        Text {
                            visible: root.subtitleRole.length > 0 && modelData[root.subtitleRole] !== undefined
                            text: modelData[root.subtitleRole] || ""
                            color: theme.textFaint
                            font.family: theme.sans
                            font.pixelSize: 9
                        }

                        Rectangle {
                            visible: root.badgeRole.length > 0 && modelData[root.badgeRole]
                            implicitWidth: badgeText.implicitWidth + 10
                            implicitHeight: 16
                            radius: 4
                            color: theme.accentSoft
                            border.color: theme.borderSoft

                            Text {
                                id: badgeText
                                anchors.centerIn: parent
                                text: {
                                    if (root.badgeRole.length === 0) return ""
                                    var val = modelData[root.badgeRole]
                                    return (typeof val === "string") ? val : (val ? "★" : "")
                                }
                                color: theme.accentStrong
                                font.family: theme.sans
                                font.pixelSize: 8
                                font.bold: true
                            }
                        }
                    }

                    MouseArea {
                        id: delegateMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: {
                            resultList.currentIndex = index
                            var filtered = root.filteredModel()
                            if (index < filtered.length) {
                                root.itemSelected(index, filtered[index])
                            }
                        }
                    }
                }
            }

            Text {
                visible: root.filteredModel().length === 0
                Layout.alignment: Qt.AlignHCenter
                text: root.filterText.length > 0 ? "No matches" : "No items"
                color: theme.textFaint
                font.family: theme.sans
                font.pixelSize: 11
            }
        }
    }
}
