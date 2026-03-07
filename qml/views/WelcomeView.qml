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
        spacing: theme.sp6
        width: Math.min(480, parent.width - theme.sp12)

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
            Layout.topMargin: theme.sp2
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
            Layout.topMargin: theme.sp4
            visible: diffController.recentRepositories.length > 0
            spacing: theme.sp2

            Text {
                text: "Recent"
                color: theme.textFaint
                font.family: theme.sans
                font.pixelSize: theme.fontSmall
                font.bold: true
            }

            Card {
                Layout.fillWidth: true
                implicitHeight: Math.min(recentList.contentHeight, 240)
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
                        radius: index === 0 ? theme.radiusLg : (index === ListView.view.count - 1 ? theme.radiusLg : 0)

                        Text {
                            anchors.fill: parent
                            anchors.leftMargin: theme.sp3
                            anchors.rightMargin: theme.sp3
                            text: modelData
                            color: theme.textBase
                            font.family: theme.mono
                            font.pixelSize: theme.fontBody
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
