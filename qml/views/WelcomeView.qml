import QtQuick
import QtQuick.Layouts
import "../components"

Rectangle {
    id: root

    signal openRepositoryRequested()
    signal openRecentRequested(string path)

    color: theme.appBg

    ColumnLayout {
        anchors.centerIn: parent
        spacing: 24
        width: Math.min(480, parent.width - 48)

        Text {
            Layout.alignment: Qt.AlignHCenter
            text: "diffy"
            color: theme.textStrong
            font.family: theme.sans
            font.pixelSize: 40
            font.bold: true
        }

        Text {
            Layout.alignment: Qt.AlignHCenter
            Layout.topMargin: 8
            text: "Native Git diff viewer"
            color: theme.textFaint
            font.family: theme.sans
            font.pixelSize: 15
        }

        ActionButton {
            Layout.alignment: Qt.AlignHCenter
            text: "Open Repository"
            tone: "accent"
            active: true
            onClicked: root.openRepositoryRequested()
        }

        ColumnLayout {
            Layout.fillWidth: true
            Layout.topMargin: 16
            visible: diffController.recentRepositories.length > 0
            spacing: 6

            Text {
                text: "Recent"
                color: theme.textFaint
                font.family: theme.sans
                font.pixelSize: 10
                font.bold: true
            }

            Rectangle {
                Layout.fillWidth: true
                implicitHeight: recentList.contentHeight
                Layout.maximumHeight: 240
                color: theme.panel
                radius: 6
                border.color: theme.borderSoft
                clip: true

                ListView {
                    id: recentList
                    anchors.fill: parent
                    model: diffController.recentRepositories
                    spacing: 0

                    delegate: Rectangle {
                        required property int index
                        required property string modelData

                        width: ListView.view.width
                        height: 36
                        color: recentMouse.containsMouse ? theme.panelStrong : "transparent"

                        Text {
                            anchors.fill: parent
                            anchors.leftMargin: 12
                            anchors.rightMargin: 12
                            text: modelData
                            color: theme.textBase
                            font.family: theme.mono
                            font.pixelSize: 12
                            elide: Text.ElideMiddle
                            verticalAlignment: Text.AlignVCenter
                        }

                        MouseArea {
                            id: recentMouse
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: root.openRecentRequested(modelData)
                        }
                    }
                }
            }
        }
    }
}
