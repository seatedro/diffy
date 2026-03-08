import QtQuick
import QtQuick.Layouts
import "../components"

Rectangle {
    id: root

    property bool compactControls: width < 1100
    property bool stackPanels: width < 800
    property bool hideFilePane: width < 600
    property string pendingPullRequestField: ""
    property real filePaneWidth: Math.max(180, Math.min(260, width * 0.20))

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

                Row {
                    spacing: theme.sp1

                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        text: repoLabel()
                        color: theme.textStrong
                        font.family: theme.sans
                        font.pixelSize: theme.fontSubtitle - 1
                        font.bold: true

                        MouseArea {
                            anchors.fill: parent
                            cursorShape: Qt.PointingHandCursor
                            onClicked: window.openRepoPicker()
                        }
                    }

                    Text {
                        visible: diffController.leftRef.length > 0
                        anchors.verticalCenter: parent.verticalCenter
                        text: "›"
                        color: theme.textFaint
                        font.family: theme.sans
                        font.pixelSize: theme.fontSubtitle - 1
                    }

                    Text {
                        visible: diffController.leftRef.length > 0
                        anchors.verticalCenter: parent.verticalCenter
                        text: diffController.leftRefDisplay + ".." + diffController.rightRefDisplay
                        color: theme.textMuted
                        font.family: theme.mono
                        font.pixelSize: theme.fontSmall

                        MouseArea {
                            anchors.fill: parent
                            cursorShape: Qt.PointingHandCursor
                            onClicked: window.openBranchPicker("left")
                        }
                    }

                    Text {
                        visible: diffController.selectedFile.path !== undefined
                        anchors.verticalCenter: parent.verticalCenter
                        text: "›"
                        color: theme.textFaint
                        font.family: theme.sans
                        font.pixelSize: theme.fontSubtitle - 1
                    }

                    Text {
                        visible: diffController.selectedFile.path !== undefined
                        anchors.verticalCenter: parent.verticalCenter
                        text: {
                            var p = diffController.selectedFile.path || ""
                            var parts = p.split("/")
                            return parts[parts.length - 1]
                        }
                        color: theme.textBase
                        font.family: theme.mono
                        font.pixelSize: theme.fontSmall
                    }
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
                    visible: !root.compactControls
                    compact: true
                    Layout.preferredWidth: 168
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
                    visible: !root.compactControls
                    text: compareModeLabel(diffController.compareMode)
                    compact: true
                    toolTip: "Toggle compare mode"
                    onClicked: diffController.compareMode = nextCompareMode(diffController.compareMode)
                }

                InputField {
                    id: rightRefField
                    visible: !root.compactControls
                    compact: true
                    Layout.preferredWidth: 168
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
                    text: diffController.wrapEnabled ? "Wrap" : "No Wrap"
                    compact: true
                    active: diffController.wrapEnabled
                    toolTip: "Toggle word wrap"
                    onClicked: diffController.wrapEnabled = !diffController.wrapEnabled
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
                visible: !root.hideFilePane
                width: root.hideFilePane ? 0 : (root.stackPanels ? parent.width : root.filePaneWidth)
                height: root.stackPanels ? 180 : parent.height
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

            SplitHandle {
                visible: !root.stackPanels && !root.hideFilePane
                x: filePane.width
                y: 0
                position: root.filePaneWidth
                minBefore: 160
                maxBefore: 400
                onDragged: function(v) { root.filePaneWidth = v }
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
                x: (root.stackPanels || root.hideFilePane) ? 0 : filePane.width + 5
                y: root.stackPanels ? filePane.height + 1 : 0
                width: (root.stackPanels || root.hideFilePane) ? parent.width : parent.width - filePane.width - 5
                height: root.stackPanels ? parent.height - filePane.height - 1 : parent.height
                fileData: diffController.selectedFile
                rowsModel: diffController.selectedFileRowsModel
                layoutMode: diffController.layoutMode
                compareGeneration: diffController.compareGeneration
                leftRef: diffController.leftRef
                rightRef: diffController.rightRef
                renderer: diffController.renderer
                wrapEnabled: diffController.wrapEnabled
                wrapColumn: diffController.wrapColumn
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
