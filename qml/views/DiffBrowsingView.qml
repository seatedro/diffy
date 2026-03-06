import QtQuick
import QtQuick.Layouts
import "../components"

Rectangle {
    id: root

    property bool compactControls: width < 1260
    property bool stackPanels: width < 1140
    property string pendingPullRequestField: ""

    function nextCompareMode(value) {
        if (value === "two-dot") return "three-dot"
        if (value === "three-dot") return "single-commit"
        return "two-dot"
    }

    function compareModeLabel(value) {
        if (value === "three-dot") return "A...B"
        if (value === "single-commit") return "Commit"
        return "A..B"
    }

    function nextRenderer(value) {
        if (!diffController.hasDifftastic) return "builtin"
        return value === "builtin" ? "difftastic" : "builtin"
    }

    function repoLabel() {
        if (diffController.repoPath.length === 0) return "No repository"
        var parts = diffController.repoPath.split("/")
        return parts.length > 0 ? parts[parts.length - 1] : diffController.repoPath
    }

    function isPullRequestUrl(value) {
        return /^https?:\/\/github\.com\/[^\/]+\/[^\/]+\/pull\/\d+(?:\/.*)?$/i.test(value.trim())
    }

    function runCompare() {
        diffController.leftRef = leftRefField.text
        diffController.rightRef = rightRefField.text
        diffController.compare()
    }

    color: theme.appBg

    Connections {
        target: diffController
        function onLeftRefChanged() { leftRefField.text = diffController.leftRef }
        function onRightRefChanged() { rightRefField.text = diffController.rightRef }
    }

    Timer {
        id: pullRequestTimer
        interval: 180
        repeat: false
        onTriggered: root.runCompare()
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 8
        spacing: 8

        Rectangle {
            Layout.fillWidth: true
            color: theme.toolbarBg
            radius: 8
            border.color: theme.borderSoft
            implicitHeight: 40

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 8
                anchors.rightMargin: 6
                anchors.topMargin: 4
                anchors.bottomMargin: 4
                spacing: 6

                ActionButton {
                    text: "←"
                    compact: true
                    onClicked: diffController.goBack()
                }

                Rectangle {
                    width: 1
                    Layout.fillHeight: true
                    Layout.topMargin: 4
                    Layout.bottomMargin: 4
                    color: theme.borderSoft
                }

                Text {
                    text: repoLabel()
                    color: theme.textStrong
                    font.family: theme.sans
                    font.pixelSize: 13
                    font.bold: true
                }

                Rectangle {
                    width: 1
                    Layout.fillHeight: true
                    Layout.topMargin: 4
                    Layout.bottomMargin: 4
                    color: theme.borderSoft
                }

                InputField {
                    id: leftRefField
                    compact: true
                    Layout.preferredWidth: root.compactControls ? 140 : 168
                    monospace: true
                    text: diffController.leftRef
                    placeholderText: "Left ref"
                    onTextChanged: {
                        if (root.isPullRequestUrl(text)) {
                            root.pendingPullRequestField = "left"
                            pullRequestTimer.restart()
                        }
                    }
                    onSubmitted: root.runCompare()
                }

                ActionButton {
                    text: compareModeLabel(diffController.compareMode)
                    compact: true
                    onClicked: diffController.compareMode = nextCompareMode(diffController.compareMode)
                }

                InputField {
                    id: rightRefField
                    compact: true
                    Layout.preferredWidth: root.compactControls ? 140 : 168
                    monospace: true
                    text: diffController.rightRef
                    placeholderText: "Right ref"
                    onTextChanged: {
                        if (root.isPullRequestUrl(text)) {
                            root.pendingPullRequestField = "right"
                            pullRequestTimer.restart()
                        }
                    }
                    onSubmitted: root.runCompare()
                }

                Item { Layout.fillWidth: true }

                Rectangle {
                    width: 1
                    Layout.fillHeight: true
                    Layout.topMargin: 4
                    Layout.bottomMargin: 4
                    color: theme.borderSoft
                }

                ActionButton {
                    text: diffController.renderer === "difftastic" ? "Difftastic" : "Built-in"
                    compact: true
                    tone: diffController.renderer === "difftastic" ? "accent" : "neutral"
                    active: diffController.renderer === "difftastic"
                    onClicked: diffController.renderer = nextRenderer(diffController.renderer)
                }

                ActionButton {
                    text: diffController.layoutMode === "split" ? "Split" : "Unified"
                    compact: true
                    active: diffController.layoutMode === "split"
                    onClicked: diffController.layoutMode = diffController.layoutMode === "unified" ? "split" : "unified"
                }

                ActionButton {
                    text: "Compare"
                    tone: "accent"
                    compact: true
                    active: true
                    onClicked: root.runCompare()
                }
            }
        }

        Rectangle {
            visible: diffController.errorMessage.length > 0
            Layout.fillWidth: true
            implicitHeight: diffErrorText.implicitHeight + 14
            radius: 6
            color: theme.dangerBg
            border.color: theme.dangerBorder

            Text {
                id: diffErrorText
                anchors.fill: parent
                anchors.margins: 8
                text: diffController.errorMessage
                color: theme.dangerText
                font.family: theme.sans
                font.pixelSize: 12
                wrapMode: Text.Wrap
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.fillHeight: true
            color: theme.panel
            radius: 8
            border.color: theme.borderSoft
            clip: true

            FileListPane {
                id: filePane
                x: 0
                y: 0
                width: root.stackPanels ? parent.width : Math.max(220, Math.min(280, parent.width * 0.20))
                height: root.stackPanels ? 220 : parent.height
                files: diffController.files
                selectedIndex: diffController.selectedFileIndex
                repoPath: diffController.repoPath
                leftRef: diffController.leftRef
                rightRef: diffController.rightRef
                compareMode: diffController.compareMode
                renderer: diffController.renderer
                onFileSelected: function(index) {
                    diffController.selectFile(index)
                }
            }

            Rectangle {
                visible: !root.stackPanels
                x: filePane.width
                y: 0
                width: 1
                height: parent.height
                color: theme.divider
            }

            Rectangle {
                visible: root.stackPanels
                x: 0
                y: filePane.height
                width: parent.width
                height: 1
                color: theme.divider
            }

            DiffPane {
                id: diffPane
                x: root.stackPanels ? 0 : filePane.width + 1
                y: root.stackPanels ? filePane.height + 1 : 0
                width: root.stackPanels ? parent.width : parent.width - filePane.width - 1
                height: root.stackPanels ? parent.height - filePane.height - 1 : parent.height
                fileData: diffController.selectedFile
                rowsModel: diffController.selectedFileRowsModel
                layoutMode: diffController.layoutMode
                leftRef: diffController.leftRef
                rightRef: diffController.rightRef
                renderer: diffController.renderer
            }
        }
    }
}
