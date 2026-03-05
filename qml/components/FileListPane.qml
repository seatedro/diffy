import QtQuick

Rectangle {
    id: root
    property var files: []
    property int selectedIndex: -1
    signal fileSelected(int index)

    color: "#131826"
    border.color: "#253048"
    radius: 12

    ListView {
        anchors.fill: parent
        anchors.margins: 8
        model: root.files
        clip: true

        delegate: Rectangle {
            required property int index
            required property var modelData
            width: ListView.view.width
            height: 48
            radius: 8
            color: root.selectedIndex === index ? "#263759" : "transparent"

            Row {
                spacing: 10
                anchors.verticalCenter: parent.verticalCenter
                anchors.left: parent.left
                anchors.leftMargin: 8

                Rectangle {
                    width: 20
                    height: 20
                    radius: 4
                    color: modelData.status === "A" ? "#214f2f" : (modelData.status === "D" ? "#5a2426" : "#2b3348")

                    Text {
                        anchors.centerIn: parent
                        text: modelData.status
                        color: "#dbe7ff"
                        font.pixelSize: 11
                        font.bold: true
                    }
                }

                Column {
                    spacing: 2
                    Text {
                        text: modelData.path
                        color: "#dbe7ff"
                        font.family: "IBM Plex Sans"
                        font.pixelSize: 13
                        elide: Text.ElideMiddle
                        width: 220
                    }
                    Text {
                        text: "+" + modelData.additions + "  -" + modelData.deletions
                        color: "#8ea4c8"
                        font.family: "JetBrains Mono"
                        font.pixelSize: 11
                    }
                }
            }

            MouseArea {
                anchors.fill: parent
                onClicked: root.fileSelected(index)
            }
        }

        Text {
            visible: root.files.length === 0
            anchors.centerIn: parent
            text: "No files"
            color: "#6d7f9f"
            font.family: "IBM Plex Sans"
        }
    }
}
