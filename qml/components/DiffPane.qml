import QtQuick

Rectangle {
    id: root
    property var fileData: ({})
    property string layoutMode: "unified"

    color: "#131826"
    border.color: "#253048"
    radius: 12

    function hasData() {
        return fileData && fileData.path !== undefined
    }

    function bgForKind(kind) {
        if (kind === "add") {
            return "#183726"
        }
        if (kind === "del") {
            return "#462024"
        }
        return "transparent"
    }

    Column {
        anchors.fill: parent
        anchors.margins: 10
        spacing: 8

        Text {
            visible: root.hasData()
            text: root.hasData() ? fileData.path : ""
            color: "#e2ebff"
            font.family: "IBM Plex Sans"
            font.bold: true
            font.pixelSize: 16
        }

        Rectangle {
            visible: root.hasData() && fileData.isBinary
            color: "#1c2436"
            border.color: "#30405f"
            radius: 8
            width: parent.width
            implicitHeight: 56

            Text {
                anchors.centerIn: parent
                text: "Binary/non-text change. Content diff unavailable."
                color: "#a7b6d4"
                font.family: "IBM Plex Sans"
            }
        }

        Flickable {
            visible: root.hasData() && !fileData.isBinary
            width: parent.width
            height: parent.height - 60
            clip: true
            contentWidth: diffColumn.width
            contentHeight: diffColumn.implicitHeight

            Column {
                id: diffColumn
                width: parent.width
                spacing: 10

                Repeater {
                    model: root.hasData() ? fileData.hunks : []

                    delegate: Column {
                        required property var modelData
                        width: parent.width
                        spacing: 4

                        Rectangle {
                            width: parent.width
                            height: 26
                            color: "#1d2a42"
                            radius: 6
                            Text {
                                anchors.left: parent.left
                                anchors.leftMargin: 8
                                anchors.verticalCenter: parent.verticalCenter
                                text: modelData.header
                                color: "#9bb0d6"
                                font.family: "JetBrains Mono"
                                font.pixelSize: 11
                            }
                        }

                        Repeater {
                            model: modelData.lines

                            delegate: Rectangle {
                                required property var modelData
                                width: parent.width
                                color: root.bgForKind(modelData.kind)
                                radius: 4
                                implicitHeight: lineText.implicitHeight + 6

                                Row {
                                    anchors.fill: parent
                                    anchors.margins: 3
                                    spacing: 8

                                    Text {
                                        text: modelData.oldLine > 0 ? modelData.oldLine : ""
                                        width: 52
                                        horizontalAlignment: Text.AlignRight
                                        color: "#7f91b4"
                                        font.family: "JetBrains Mono"
                                        font.pixelSize: 12
                                    }

                                    Text {
                                        text: modelData.newLine > 0 ? modelData.newLine : ""
                                        width: 52
                                        horizontalAlignment: Text.AlignRight
                                        color: "#7f91b4"
                                        font.family: "JetBrains Mono"
                                        font.pixelSize: 12
                                    }

                                    Text {
                                        id: lineText
                                        text: root.layoutMode === "split" && modelData.kind === "del" ? modelData.text + "    |" : modelData.text
                                        color: modelData.kind === "ctx" ? "#d7e1f6" : "#eaf0ff"
                                        font.family: "JetBrains Mono"
                                        font.pixelSize: 12
                                        wrapMode: Text.WrapAnywhere
                                        width: parent.width - 130
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Text {
            visible: !root.hasData()
            anchors.horizontalCenter: parent.horizontalCenter
            text: "Select refs and click Compare"
            color: "#7a8cae"
            font.family: "IBM Plex Sans"
        }
    }
}
