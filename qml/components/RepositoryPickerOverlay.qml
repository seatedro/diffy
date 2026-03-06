import QtQuick
import QtQuick.Layouts

Item {
    id: root

    visible: diffController.repositoryPickerVisible

    onVisibleChanged: {
        if (visible) {
            pickerList.currentIndex = 0
            pickerPopover.forceActiveFocus()
        }
    }

    Rectangle {
        anchors.fill: parent
        color: "#80000000"
        opacity: root.visible ? 1.0 : 0.0
        Behavior on opacity { NumberAnimation { duration: 120 } }
    }

    MouseArea {
        anchors.fill: parent
        onClicked: function(mouse) {
            if (!pickerPopover.contains(Qt.point(mouse.x, mouse.y))) {
                diffController.closeRepositoryPicker()
            }
        }
    }

    Rectangle {
        id: pickerPopover
        property real popupWidth: Math.min(parent.width - 24, 400)
        property real popupHeight: Math.min(parent.height - 24, 400)
        anchors.centerIn: parent
        width: popupWidth
        height: popupHeight
        radius: 10
        color: theme.toolbarBg
        border.color: theme.borderStrong
        focus: diffController.repositoryPickerVisible

        layer.enabled: true
        layer.effect: null

        Keys.onPressed: function(event) {
            if (event.key === Qt.Key_Escape) {
                diffController.closeRepositoryPicker()
                event.accepted = true
                return
            }
            if (event.key === Qt.Key_Up) {
                pickerList.decrementCurrentIndex()
                event.accepted = true
                return
            }
            if (event.key === Qt.Key_Down) {
                pickerList.incrementCurrentIndex()
                event.accepted = true
                return
            }
            if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                if (diffController.repositoryPickerModel.currentPathIsRepository && pickerList.currentIndex < 0) {
                    diffController.openCurrentRepositoryFromPicker()
                } else if (pickerList.currentIndex >= 0) {
                    diffController.activateRepositoryPickerEntry(pickerList.currentIndex)
                }
                event.accepted = true
            }
        }

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: 10
            spacing: 6

            Text {
                text: "Open Repository"
                color: theme.textStrong
                font.family: theme.sans
                font.pixelSize: 12
                font.bold: true
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 6

                ActionButton {
                    text: "Up"
                    compact: true
                    onClicked: diffController.navigateRepositoryPickerUp()
                }

                Rectangle {
                    Layout.fillWidth: true
                    implicitHeight: 26
                    radius: 4
                    color: theme.panel
                    border.color: theme.borderSoft

                    Text {
                        anchors.fill: parent
                        anchors.leftMargin: 8
                        anchors.rightMargin: 8
                        text: diffController.repositoryPickerModel.currentPath
                        color: theme.textFaint
                        font.family: theme.mono
                        font.pixelSize: 8
                        elide: Text.ElideMiddle
                        verticalAlignment: Text.AlignVCenter
                    }
                }
            }

            Rectangle {
                Layout.fillWidth: true
                visible: diffController.repositoryPickerModel.currentPathIsRepository
                implicitHeight: 28
                radius: 4
                color: openCurrentMouse.containsMouse || pickerList.currentIndex < 0 ? theme.panelStrong : theme.panel

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 8
                    anchors.rightMargin: 8
                    spacing: 8

                    Text {
                        text: "open this directory"
                        color: theme.textBase
                        font.family: theme.sans
                        font.pixelSize: 10
                        Layout.fillWidth: true
                    }

                    Rectangle {
                        implicitWidth: 36
                        implicitHeight: 16
                        radius: 4
                        color: theme.accentSoft
                        border.color: theme.borderSoft

                        Text {
                            anchors.centerIn: parent
                            text: "repo"
                            color: theme.accentStrong
                            font.family: theme.sans
                            font.pixelSize: 8
                            font.bold: true
                        }
                    }
                }

                MouseArea {
                    id: openCurrentMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    onClicked: diffController.openCurrentRepositoryFromPicker()
                }
            }

            ListView {
                id: pickerList
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                spacing: 0
                model: diffController.repositoryPickerModel
                currentIndex: diffController.repositoryPickerModel.currentPathIsRepository ? -1 : 0

                delegate: Rectangle {
                    required property int index
                    required property string name
                    required property string path
                    required property bool isRepository

                    width: ListView.view.width
                    height: 26
                    color: mouseArea.containsMouse || pickerList.currentIndex === index ? theme.panelStrong : "transparent"

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 8
                        anchors.rightMargin: 8
                        spacing: 8

                        Text {
                            text: name
                            color: theme.textBase
                            font.family: theme.sans
                            font.pixelSize: 10
                            Layout.fillWidth: true
                            elide: Text.ElideRight
                        }

                        Rectangle {
                            visible: isRepository
                            implicitWidth: 34
                            implicitHeight: 16
                            radius: 4
                            color: theme.accentSoft
                            border.color: theme.borderSoft

                            Text {
                                anchors.centerIn: parent
                                text: "repo"
                                color: theme.accentStrong
                                font.family: theme.sans
                                font.pixelSize: 8
                                font.bold: true
                            }
                        }
                    }

                    MouseArea {
                        id: mouseArea
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: {
                            pickerList.currentIndex = index
                            diffController.activateRepositoryPickerEntry(index)
                        }
                    }
                }
            }
        }
    }
}
