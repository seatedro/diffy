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
    property string pendingPullRequestField: ""
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

    function isPullRequestUrl(value) {
        var trimmed = value.trim()
        return /^https?:\/\/github\.com\/[^\/]+\/[^\/]+\/pull\/\d+(?:\/.*)?$/i.test(trimmed)
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

    Timer {
        id: pullRequestTimer
        interval: 180
        repeat: false
        onTriggered: runCompare()
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
                    id: repoPanel
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
                            onClicked: diffController.openRepositoryPicker()
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
                    placeholderText: "Left ref or PR URL"
                    onTextChanged: {
                        if (window.isPullRequestUrl(text)) {
                            window.pendingPullRequestField = "left"
                            pullRequestTimer.restart()
                        }
                    }
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
                    placeholderText: "Right ref or PR URL"
                    onTextChanged: {
                        if (window.isPullRequestUrl(text)) {
                            window.pendingPullRequestField = "right"
                            pullRequestTimer.restart()
                        }
                    }
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

    Item {
        visible: diffController.repositoryPickerVisible
        anchors.fill: parent
        onVisibleChanged: {
            if (visible) {
                pickerList.currentIndex = 0
                pickerPopover.forceActiveFocus()
            }
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
            property point anchorPoint: repoPanel.mapToItem(window.contentItem, 0, repoPanel.height + 6)
            property real popupWidth: Math.min(parent.width - 24, 360)
            property real popupHeight: Math.min(parent.height - (window.chromeMargin * 2) - 40, 320)
            x: Math.max(window.chromeMargin, Math.min(anchorPoint.x, parent.width - popupWidth - window.chromeMargin))
            y: Math.max(window.chromeMargin + 34,
                        Math.min(anchorPoint.y, parent.height - popupHeight - window.chromeMargin))
            width: popupWidth
            height: popupHeight
            radius: 8
            color: theme.toolbarBg
            border.color: theme.borderStrong
            focus: diffController.repositoryPickerVisible

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
                anchors.margins: 8
                spacing: 6

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 6

                    ActionButton {
                        theme: theme
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
}
