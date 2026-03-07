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
        function onLeftRefChanged() { leftRefField.text = diffController.leftRefDisplay }
        function onRightRefChanged() { rightRefField.text = diffController.rightRefDisplay }
        function onCurrentViewChanged() {
            if (diffController.currentView === "diff")
                diffPane.focusSurface()
        }
        function onSelectedFileIndexChanged() {
            if (diffController.currentView === "diff")
                diffPane.focusSurface()
        }
    }

    Timer {
        id: pullRequestTimer
        interval: 180
        repeat: false
        onTriggered: root.runCompare()
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: theme.sp2
        spacing: theme.sp2

        // --- Toolbar ---
        Rectangle {
            Layout.fillWidth: true
            color: theme.toolbarBg
            radius: theme.radiusLg
            border.color: theme.borderSoft
            implicitHeight: 48

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: theme.sp3
                anchors.rightMargin: theme.sp3
                anchors.topMargin: theme.sp1
                anchors.bottomMargin: theme.sp1
                spacing: theme.sp2

                ActionButton {
                    text: "Back"
                    toolTip: "Return to compare setup"
                    onClicked: diffController.goBack()
                }

                Rectangle {
                    width: 1
                    Layout.fillHeight: true
                    Layout.topMargin: theme.sp1
                    Layout.bottomMargin: theme.sp1
                    color: theme.borderSoft
                }

                Text {
                    text: repoLabel()
                    color: theme.textStrong
                    font.family: theme.sans
                    font.pixelSize: theme.fontSubtitle - 1
                    font.bold: true
                }

                Rectangle {
                    width: 1
                    Layout.fillHeight: true
                    Layout.topMargin: theme.sp1
                    Layout.bottomMargin: theme.sp1
                    color: theme.borderSoft
                }

                InputField {
                    id: leftRefField
                    compact: true
                    Layout.preferredWidth: root.compactControls ? 140 : 168
                    monospace: true
                    text: diffController.leftRefDisplay
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
                    toolTip: "Toggle compare mode"
                    onClicked: diffController.compareMode = nextCompareMode(diffController.compareMode)
                }

                InputField {
                    id: rightRefField
                    compact: true
                    Layout.preferredWidth: root.compactControls ? 140 : 168
                    monospace: true
                    text: diffController.rightRefDisplay
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
                    Layout.topMargin: theme.sp1
                    Layout.bottomMargin: theme.sp1
                    color: theme.borderSoft
                }

                SegmentedControl {
                    options: [
                        {label: "Built-in", value: "builtin"},
                        {label: "Difftastic", value: "difftastic"}
                    ]
                    currentValue: diffController.renderer
                    onValueChanged: function(v) { diffController.renderer = v }
                }

                SegmentedControl {
                    options: [
                        {label: "Unified", value: "unified"},
                        {label: "Split", value: "split"}
                    ]
                    currentValue: diffController.layoutMode
                    onValueChanged: function(v) { diffController.layoutMode = v }
                }

                ActionButton {
                    text: diffController.comparing ? "Comparing…" : "Compare"
                    tone: "accent"
                    compact: true
                    active: true
                    toolTip: "Run comparison"
                    onClicked: root.runCompare()
                }
            }
        }

        // --- Error banner ---
        Rectangle {
            visible: diffController.errorMessage.length > 0
            Layout.fillWidth: true
            implicitHeight: diffErrorText.implicitHeight + theme.sp4
            radius: theme.radiusMd
            color: theme.dangerBg
            border.color: theme.dangerBorder

            Text {
                id: diffErrorText
                anchors.fill: parent
                anchors.margins: theme.sp2
                text: diffController.errorMessage
                color: theme.dangerText
                font.family: theme.sans
                font.pixelSize: theme.fontBody
                wrapMode: Text.Wrap
            }
        }

        // --- Main content ---
        Rectangle {
            Layout.fillWidth: true
            Layout.fillHeight: true
            color: theme.panel
            radius: theme.radiusLg
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
                onNextFileRequested: {
                    var next = diffController.selectedFileIndex + 1
                    if (next < diffController.files.length)
                        diffController.selectFile(next)
                }
                onPreviousFileRequested: {
                    var prev = diffController.selectedFileIndex - 1
                    if (prev >= 0)
                        diffController.selectFile(prev)
                }
            }
        }
    }
}
