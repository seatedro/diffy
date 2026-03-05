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
    property int chromeMargin: 8
    property int chromeGap: 8

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
            radius: 10
            border.color: theme.borderSoft
            implicitHeight: headerColumn.implicitHeight + 18

            ColumnLayout {
                id: headerColumn
                anchors.fill: parent
                anchors.margins: 8
                spacing: 8

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 10

                    Column {
                        spacing: 0

                        Text {
                            text: "LOCAL GIT REVIEW"
                            color: theme.textFaint
                            font.family: theme.sans
                            font.pixelSize: 9
                            font.bold: true
                            font.letterSpacing: 1.2
                        }

                        Text {
                            text: "diffy"
                            color: theme.textStrong
                            font.family: theme.sans
                            font.pixelSize: 20
                            font.bold: true
                        }
                    }

                    Rectangle {
                        width: repoChipLabel.implicitWidth + 18
                        height: 22
                        radius: 6
                        color: theme.panelTint
                        border.color: theme.borderSoft

                        Text {
                            id: repoChipLabel
                            anchors.centerIn: parent
                            text: repoLabel()
                            color: theme.textBase
                            font.family: theme.sans
                            font.pixelSize: 11
                            font.bold: true
                        }
                    }

                    Item {
                        Layout.fillWidth: true
                    }

                    Row {
                        spacing: 8

                        Rectangle {
                            width: modeChip.implicitWidth + 20
                            height: 24
                            radius: 6
                            color: theme.panelTint
                            border.color: theme.borderSoft

                            Text {
                                id: modeChip
                                anchors.centerIn: parent
                                text: compareModeLabel(diffController.compareMode)
                                color: theme.textMuted
                                font.family: theme.mono
                                font.pixelSize: 10
                            }
                        }

                        Rectangle {
                            width: rendererChip.implicitWidth + 20
                            height: 24
                            radius: 6
                            color: diffController.renderer === "difftastic" ? theme.warningBg : theme.accentSoft
                            border.color: diffController.renderer === "difftastic" ? theme.warningBorder : theme.borderSoft

                            Text {
                                id: rendererChip
                                anchors.centerIn: parent
                                text: diffController.renderer === "difftastic" ? "difftastic" : "built-in"
                                color: diffController.renderer === "difftastic" ? theme.warningText : theme.accentStrong
                                font.family: theme.sans
                                font.pixelSize: 10
                                font.bold: true
                            }
                        }
                    }
                }

                Rectangle {
                    Layout.fillWidth: true
                    height: 1
                    color: theme.divider
                }

                Flow {
                    Layout.fillWidth: true
                    spacing: 8

                    InputField {
                        id: repoField
                        theme: theme
                        width: window.compactControls ? parent.width : Math.max(360, parent.width - 202)
                        text: diffController.repoPath
                        placeholderText: "Local git repository path"
                        onSubmitted: diffController.openRepository(text)
                    }

                    ActionButton {
                        theme: theme
                        text: "Open Repo"
                        tone: "success"
                        onClicked: diffController.openRepository(repoField.text)
                    }

                    ActionButton {
                        theme: theme
                        text: "Compare"
                        tone: "accent"
                        active: true
                        onClicked: runCompare()
                    }
                }

                Flow {
                    Layout.fillWidth: true
                    spacing: 8

                    InputField {
                        id: leftRefField
                        theme: theme
                        width: window.compactControls ? Math.max(220, (parent.width - 10) / 2) : 248
                        monospace: true
                        text: diffController.leftRef
                        placeholderText: "Left ref"
                        onSubmitted: diffController.leftRef = text
                    }

                    InputField {
                        id: rightRefField
                        theme: theme
                        width: window.compactControls ? Math.max(220, (parent.width - 10) / 2) : 248
                        monospace: true
                        text: diffController.rightRef
                        placeholderText: "Right ref"
                        onSubmitted: diffController.rightRef = text
                    }

                    ActionButton {
                        theme: theme
                        text: compareModeLabel(diffController.compareMode)
                        compact: true
                        active: true
                        onClicked: diffController.compareMode = nextCompareMode(diffController.compareMode)
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
                        tone: "neutral"
                        active: diffController.layoutMode === "split"
                        onClicked: diffController.layoutMode = diffController.layoutMode === "unified" ? "split" : "unified"
                    }
                }
            }
        }

        Rectangle {
            visible: diffController.refs.length > 0
            Layout.fillWidth: true
            color: theme.panel
            radius: 6
            border.color: theme.borderSoft
            implicitHeight: 34

            Row {
                anchors.fill: parent
                anchors.margins: 5
                spacing: 8

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: "refs"
                    color: theme.textFaint
                    font.family: theme.sans
                    font.pixelSize: 11
                    font.bold: true
                }

                ListView {
                    anchors.verticalCenter: parent.verticalCenter
                    width: parent.width - 80
                    height: 24
                    orientation: ListView.Horizontal
                    spacing: 8
                    clip: true
                    model: diffController.refs
                    boundsBehavior: Flickable.StopAtBounds

                    delegate: Rectangle {
                        required property var modelData
                        width: chipText.implicitWidth + 20
                        height: 24
                        radius: 4
                        color: leftRefField.text === modelData || rightRefField.text === modelData ? theme.accentSoft : theme.panelStrong
                        border.color: leftRefField.text === modelData || rightRefField.text === modelData ? theme.selectionBorder : theme.borderSoft

                        Text {
                            id: chipText
                            anchors.centerIn: parent
                            text: modelData
                            color: leftRefField.text === modelData || rightRefField.text === modelData ? theme.accentStrong : theme.textMuted
                            font.family: theme.mono
                            font.pixelSize: 10
                        }

                        MouseArea {
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: assignQuickRef(modelData)
                        }
                    }
                }
            }
        }

        Rectangle {
            visible: diffController.errorMessage.length > 0
            Layout.fillWidth: true
            implicitHeight: errorText.implicitHeight + 18
            radius: 6
            color: theme.dangerBg
            border.color: theme.dangerBorder

            Text {
                id: errorText
                anchors.fill: parent
                anchors.margins: 10
                text: diffController.errorMessage
                color: theme.dangerText
                font.family: theme.sans
                font.pixelSize: 13
                wrapMode: Text.Wrap
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.fillHeight: true
            color: theme.panel
            radius: 10
            border.color: theme.borderSoft

            FileListPane {
                id: filePane
                theme: theme
                x: 0
                y: 0
                width: window.stackPanels ? parent.width : Math.max(260, Math.min(300, parent.width * 0.22))
                height: window.stackPanels ? 268 : parent.height
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
                rowsModel: diffController.selectedFileRows
                layoutMode: diffController.layoutMode
                leftRef: diffController.leftRef
                rightRef: diffController.rightRef
                renderer: diffController.renderer
            }
        }
    }
}
