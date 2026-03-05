import QtQuick
import QtQuick.Layouts
import QtQuick.Window
import "components"

Window {
    id: window
    visible: true
    width: 1540
    height: 940
    title: "diffy"
    color: "#0b0e14"

    function nextCompareMode(value) {
        if (value === "two-dot") {
            return "three-dot"
        }
        if (value === "three-dot") {
            return "single-commit"
        }
        return "two-dot"
    }

    function compareModeLabel(value) {
        if (value === "three-dot") {
            return "A...B"
        }
        if (value === "single-commit") {
            return "Commit"
        }
        return "A..B"
    }

    function nextRenderer(value) {
        if (!diffController.hasDifftastic) {
            return "builtin"
        }
        return value === "builtin" ? "difftastic" : "builtin"
    }

    Rectangle {
        anchors.fill: parent
        gradient: Gradient {
            GradientStop { position: 0.0; color: "#101723" }
            GradientStop { position: 0.5; color: "#0b0e14" }
            GradientStop { position: 1.0; color: "#090c12" }
        }
    }

    Connections {
        target: diffController

        function onRepoPathChanged() {
            repoField.text = diffController.repoPath
        }

        function onLeftRefChanged() {
            leftRefField.text = diffController.leftRef
        }

        function onRightRefChanged() {
            rightRefField.text = diffController.rightRef
        }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 16
        spacing: 12

        Rectangle {
            Layout.fillWidth: true
            radius: 18
            color: "#121a29"
            border.color: "#22304a"
            implicitHeight: 132

            ColumnLayout {
                anchors.fill: parent
                anchors.margins: 14
                spacing: 10

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 10

                    InputField {
                        id: repoField
                        Layout.fillWidth: true
                        text: diffController.repoPath
                        placeholderText: "Local git repository path"
                        onSubmitted: diffController.openRepository(text)
                    }

                    ActionButton {
                        text: "Open Repo"
                        emphasized: true
                        fillColor: "#1e4b38"
                        hoverColor: "#28654b"
                        onClicked: diffController.openRepository(repoField.text)
                    }
                }

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 10

                    InputField {
                        id: leftRefField
                        Layout.fillWidth: true
                        monospace: true
                        text: diffController.leftRef
                        placeholderText: "Left ref"
                        onSubmitted: diffController.leftRef = text
                    }

                    InputField {
                        id: rightRefField
                        Layout.fillWidth: true
                        monospace: true
                        text: diffController.rightRef
                        placeholderText: "Right ref"
                        onSubmitted: diffController.rightRef = text
                    }

                    ActionButton {
                        text: compareModeLabel(diffController.compareMode)
                        onClicked: diffController.compareMode = nextCompareMode(diffController.compareMode)
                    }

                    ActionButton {
                        text: diffController.renderer === "difftastic" ? "Difftastic" : "Built-in"
                        onClicked: diffController.renderer = nextRenderer(diffController.renderer)
                    }

                    ActionButton {
                        text: diffController.layoutMode === "split" ? "Split" : "Unified"
                        onClicked: diffController.layoutMode = diffController.layoutMode === "unified" ? "split" : "unified"
                    }

                    ActionButton {
                        text: "Compare"
                        emphasized: true
                        fillColor: "#345ea8"
                        hoverColor: "#4677c9"
                        onClicked: {
                            diffController.leftRef = leftRefField.text
                            diffController.rightRef = rightRefField.text
                            diffController.compare()
                        }
                    }
                }

                Flickable {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 28
                    clip: true
                    contentWidth: refRow.implicitWidth
                    contentHeight: refRow.implicitHeight

                    Row {
                        id: refRow
                        spacing: 6

                        Repeater {
                            model: diffController.refs

                            delegate: Rectangle {
                                required property var modelData
                                radius: 7
                                height: 24
                                width: refLabel.implicitWidth + 16
                                color: "#172233"
                                border.color: "#2a3b58"

                                Text {
                                    id: refLabel
                                    anchors.centerIn: parent
                                    text: modelData
                                    color: "#b5c6e8"
                                    font.family: "JetBrains Mono"
                                    font.pixelSize: 11
                                }

                                MouseArea {
                                    anchors.fill: parent
                                    onClicked: {
                                        if (leftRefField.text.length === 0 || leftRefField.text === diffController.leftRef) {
                                            leftRefField.text = modelData
                                            diffController.leftRef = modelData
                                        } else {
                                            rightRefField.text = modelData
                                            diffController.rightRef = modelData
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Rectangle {
            visible: diffController.errorMessage.length > 0
            Layout.fillWidth: true
            color: "#3d1c1f"
            radius: 10
            border.color: "#82404b"
            implicitHeight: errorText.implicitHeight + 16

            Text {
                id: errorText
                anchors.fill: parent
                anchors.margins: 10
                text: diffController.errorMessage
                color: "#ffc8c8"
                wrapMode: Text.Wrap
                font.family: "IBM Plex Sans"
                font.pixelSize: 13
            }
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 12

            FileListPane {
                Layout.fillHeight: true
                Layout.preferredWidth: 320
                files: diffController.files
                selectedIndex: diffController.selectedFileIndex
                onFileSelected: diffController.selectFile(index)
            }

            DiffPane {
                Layout.fillWidth: true
                Layout.fillHeight: true
                fileData: diffController.selectedFile()
                layoutMode: diffController.layoutMode
            }
        }
    }
}
