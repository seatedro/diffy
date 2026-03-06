import QtQuick
import QtQuick.Layouts
import QtQuick.Window
import "components"

Window {
    id: window
    visible: true
    width: 1540
    height: 940
    minimumWidth: 960
    minimumHeight: 720
    title: "diffy"
    color: theme.appBg

    property bool compactControls: width < 1260
    property bool stackPanels: width < 1140
    property bool editingRepoPath: false
    property int chromeMargin: 6
    property int chromeGap: 6

    QtObject {
        id: theme

        readonly property string sans: "IBM Plex Sans"
        readonly property string mono: "JetBrains Mono"

        readonly property color appBg: "#1d2021"
        readonly property color canvas: "#282828"
        readonly property color panel: "#32302f"
        readonly property color panelStrong: "#3c3836"
        readonly property color panelTint: "#504945"
        readonly property color toolbarBg: "#282828"

        readonly property color borderSoft: "#504945"
        readonly property color borderStrong: "#665c54"
        readonly property color divider: "#504945"

        readonly property color textStrong: "#fbf1c7"
        readonly property color textBase: "#ebdbb2"
        readonly property color textMuted: "#d5c4a1"
        readonly property color textFaint: "#a89984"

        readonly property color accent: "#83a598"
        readonly property color accentStrong: "#83a598"
        readonly property color accentSoft: "#3b4b4f"

        readonly property color successBg: "#32361a"
        readonly property color successBorder: "#4a5a1c"
        readonly property color successText: "#b8bb26"

        readonly property color dangerBg: "#3c1f1e"
        readonly property color dangerBorder: "#7c3a31"
        readonly property color dangerText: "#fb4934"

        readonly property color warningBg: "#4a3b16"
        readonly property color warningBorder: "#7c6f27"
        readonly property color warningText: "#fabd2f"

        readonly property color selectionBg: "#3c3836"
        readonly property color selectionBorder: "#83a598"

        readonly property color lineContext: "#282828"
        readonly property color lineContextAlt: "#232323"
        readonly property color lineAdd: "#2d3216"
        readonly property color lineAddAccent: "#32361a"
        readonly property color lineDel: "#3c1f1e"
        readonly property color lineDelAccent: "#442624"
    }

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

    function repoLabel() {
        if (diffController.repoPath.length === 0) {
            return "No repository"
        }
        var parts = diffController.repoPath.split("/")
        return parts.length > 0 ? parts[parts.length - 1] : diffController.repoPath
    }

    function runCompare() {
        diffController.leftRef = leftRefField.text
        diffController.rightRef = rightRefField.text
        diffController.compare()
    }

    function submitRepoPath() {
        var nextPath = repoField.text.trim()
        if (nextPath.length === 0) {
            return
        }
        if (diffController.openRepository(nextPath)) {
            window.editingRepoPath = false
        }
    }

    function assignQuickRef(refName) {
        if (leftRefField.activeFocus || leftRefField.text.length === 0) {
            leftRefField.text = refName
            diffController.leftRef = refName
            return
        }
        if (rightRefField.activeFocus || rightRefField.text.length === 0) {
            rightRefField.text = refName
            diffController.rightRef = refName
            return
        }
        rightRefField.text = refName
        diffController.rightRef = refName
    }

    Rectangle {
        anchors.fill: parent
        color: theme.appBg
    }

    Connections {
        target: diffController

        function onRepoPathChanged() {
            repoField.text = diffController.repoPath
            window.editingRepoPath = false
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
        anchors.margins: window.chromeMargin
        spacing: window.chromeGap

        Rectangle {
            Layout.fillWidth: true
            color: theme.toolbarBg
            radius: 8
            border.color: theme.borderSoft
            implicitHeight: 40

            RowLayout {
                anchors.fill: parent
                anchors.margins: 5
                spacing: 6

                Text {
                    text: "diffy"
                    color: theme.textStrong
                    font.family: theme.sans
                    font.pixelSize: 17
                    font.bold: true
                }

                Rectangle {
                    Layout.preferredWidth: window.compactControls ? 320 : 420
                    Layout.fillWidth: true
                    implicitHeight: 28
                    radius: 5
                    color: theme.panel
                    border.color: window.editingRepoPath ? theme.selectionBorder : theme.borderSoft

                    RowLayout {
                        anchors.fill: parent
                        anchors.margins: 5
                        spacing: 6

                        Rectangle {
                            Layout.preferredWidth: repoChipLabel.implicitWidth + 14
                            implicitHeight: 16
                            radius: 4
                            color: theme.panelTint
                            border.color: theme.borderSoft

                            Text {
                                id: repoChipLabel
                                anchors.centerIn: parent
                                text: repoLabel()
                                color: theme.textBase
                                font.family: theme.sans
                                font.pixelSize: 9
                                font.bold: true
                            }
                        }

                        Item {
                            Layout.fillWidth: true
                            Layout.preferredHeight: 18

                            Text {
                                anchors.fill: parent
                                visible: !window.editingRepoPath
                                text: diffController.repoPath.length > 0 ? diffController.repoPath : "Choose a local repository"
                                color: diffController.repoPath.length > 0 ? theme.textMuted : theme.textFaint
                                font.family: theme.mono
                                font.pixelSize: 9
                                elide: Text.ElideMiddle
                                verticalAlignment: Text.AlignVCenter
                            }

                            TextInput {
                                id: repoField
                                anchors.fill: parent
                                visible: window.editingRepoPath
                                text: diffController.repoPath
                                color: theme.textStrong
                                font.family: theme.mono
                                font.pixelSize: 9
                                clip: true
                                selectByMouse: true
                                selectedTextColor: "#ffffff"
                                selectionColor: theme.accent
                                onAccepted: submitRepoPath()
                            }
                        }

                        ActionButton {
                            visible: !window.editingRepoPath
                            theme: theme
                            text: "Edit"
                            compact: true
                            onClicked: {
                                window.editingRepoPath = true
                                repoField.forceActiveFocus()
                                repoField.selectAll()
                            }
                        }

                        ActionButton {
                            visible: window.editingRepoPath
                            theme: theme
                            text: "Open"
                            compact: true
                            tone: "success"
                            onClicked: submitRepoPath()
                        }

                        ActionButton {
                            visible: window.editingRepoPath
                            theme: theme
                            text: "Cancel"
                            compact: true
                            onClicked: {
                                repoField.text = diffController.repoPath
                                window.editingRepoPath = false
                            }
                        }

                        ActionButton {
                            theme: theme
                            text: "Browse"
                            compact: true
                            onClicked: {
                                if (diffController.chooseRepositoryAndOpen()) {
                                    repoField.text = diffController.repoPath
                                }
                            }
                        }
                    }
                }

                InputField {
                    id: leftRefField
                    theme: theme
                    compact: true
                    Layout.preferredWidth: window.compactControls ? 150 : 172
                    monospace: true
                    text: diffController.leftRef
                    placeholderText: "Left ref"
                    onSubmitted: runCompare()
                }

                ActionButton {
                    theme: theme
                    text: compareModeLabel(diffController.compareMode)
                    compact: true
                    onClicked: diffController.compareMode = nextCompareMode(diffController.compareMode)
                }

                InputField {
                    id: rightRefField
                    theme: theme
                    compact: true
                    Layout.preferredWidth: window.compactControls ? 150 : 172
                    monospace: true
                    text: diffController.rightRef
                    placeholderText: "Right ref"
                    onSubmitted: runCompare()
                }

                ActionButton {
                    theme: theme
                    text: diffController.renderer === "difftastic" ? "Difftastic" : "Built-in"
                    compact: true
                    tone: diffController.renderer === "difftastic" ? "accent" : "neutral"
                    active: diffController.renderer === "difftastic"
                    onClicked: diffController.renderer = nextRenderer(diffController.renderer)
                }

                ActionButton {
                    theme: theme
                    text: diffController.layoutMode === "split" ? "Split" : "Unified"
                    compact: true
                    active: diffController.layoutMode === "split"
                    onClicked: diffController.layoutMode = diffController.layoutMode === "unified" ? "split" : "unified"
                }

                ActionButton {
                    theme: theme
                    text: "Compare"
                    tone: "accent"
                    compact: true
                    active: true
                    onClicked: runCompare()
                }
            }
        }

        Rectangle {
            visible: diffController.errorMessage.length > 0
            Layout.fillWidth: true
            implicitHeight: errorText.implicitHeight + 14
            radius: 6
            color: theme.dangerBg
            border.color: theme.dangerBorder

            Text {
                id: errorText
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

            FileListPane {
                id: filePane
                theme: theme
                x: 0
                y: 0
                width: window.stackPanels ? parent.width : Math.max(196, Math.min(224, parent.width * 0.18))
                height: window.stackPanels ? 220 : parent.height
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
                visible: !window.stackPanels
                x: filePane.width
                y: 0
                width: 1
                height: parent.height
                color: theme.divider
            }

            Rectangle {
                visible: window.stackPanels
                x: 0
                y: filePane.height
                width: parent.width
                height: 1
                color: theme.divider
            }

            DiffPane {
                id: diffPane
                theme: theme
                x: window.stackPanels ? 0 : filePane.width + 1
                y: window.stackPanels ? filePane.height + 1 : 0
                width: window.stackPanels ? parent.width : parent.width - filePane.width - 1
                height: window.stackPanels ? parent.height - filePane.height - 1 : parent.height
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
