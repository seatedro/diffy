import QtQuick
import QtQuick.Layouts
import QtQuick.Window
import "../components"

Rectangle {
    id: root

    property alias surfaceItem: diffPane.surfaceItem
    property bool compactControls: width < 1100
    property bool stackPanels: width < 800
    property bool sidebarManuallyHidden: false
    property bool hideFilePane: width < 600 || sidebarManuallyHidden
    property real filePaneWidth: Math.max(180, Math.min(260, width * 0.20))

    readonly property string iconArrowLeft: '<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m12 19-7-7 7-7"/><path d="M19 12H5"/></svg>'
    readonly property string iconGitBranch: '<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M15 6a9 9 0 0 0-9 9V3"/><circle cx="18" cy="6" r="3"/><circle cx="6" cy="18" r="3"/></svg>'
    readonly property string iconArrowRightLeft: '<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m16 3 4 4-4 4"/><path d="M20 7H4"/><path d="m8 21-4-4 4-4"/><path d="M4 17h16"/></svg>'
    readonly property string iconChevronRight: '<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>'
    readonly property string iconCommand: '<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M15 6v12a3 3 0 1 0 3-3H6a3 3 0 1 0 3 3V6a3 3 0 1 0-3 3h12a3 3 0 1 0-3-3"/></svg>'

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

    function compareModeIcon(value) {
        if (value === "three-dot") return "..."
        if (value === "single-commit") return "@"
        return ".."
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

    function runCompare() {
        diffController.compare()
    }

    function toggleSidebar() {
        sidebarManuallyHidden = !sidebarManuallyHidden
    }

    function swapRefs() {
        var tmpLeft = diffController.leftRef
        diffController.leftRef = diffController.rightRef
        diffController.rightRef = tmpLeft
    }

    color: theme.appBg

    Connections {
        target: diffController
        function onCurrentViewChanged() {
            if (diffController.currentView === "diff")
                diffPane.focusSurface()
        }
        function onSelectedFileIndexChanged() {
            if (diffController.currentView === "diff")
                diffPane.focusSurface()
        }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: theme.sp2
        spacing: 0

        // ──────────────────────────────────────────────────
        // Row 1 — Navigation breadcrumb strip
        // ──────────────────────────────────────────────────
        Item {
            Layout.fillWidth: true
            implicitHeight: 28

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: theme.sp1
                anchors.rightMargin: theme.sp2
                spacing: 4

                Icon {
                    svg: root.iconArrowLeft
                    size: 14
                    color: navBackMouse.containsMouse ? theme.textMuted : theme.textFaint
                    Behavior on color {
                        enabled: !(root.Window.window && root.Window.window.commandPaletteShowing)
                        ColorAnimation { duration: 35 }
                    }

                    MouseArea {
                        id: navBackMouse
                        anchors.fill: parent
                        anchors.margins: -6
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: diffController.goBack()
                    }
                }

                Text {
                    text: repoLabel()
                    color: repoMouse.containsMouse ? theme.textMuted : theme.textFaint
                    font.family: theme.sans
                    font.pixelSize: theme.fontSmall
                    Behavior on color {
                        enabled: !(root.Window.window && root.Window.window.commandPaletteShowing)
                        ColorAnimation { duration: 35 }
                    }

                    MouseArea {
                        id: repoMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: window.openRepoPicker()
                    }
                }

                Icon {
                    visible: diffController.leftRef.length > 0
                    svg: root.iconChevronRight
                    size: 10
                    color: theme.textFaint
                    opacity: 0.5
                }

                Text {
                    visible: diffController.leftRef.length > 0
                    text: diffController.leftRefDisplay + compareModeIcon(diffController.compareMode) + diffController.rightRefDisplay
                    color: refCrumbMouse.containsMouse ? theme.textMuted : theme.textFaint
                    font.family: theme.mono
                    font.pixelSize: theme.fontCaption + 1
                    Behavior on color {
                        enabled: !(root.Window.window && root.Window.window.commandPaletteShowing)
                        ColorAnimation { duration: 35 }
                    }

                    MouseArea {
                        id: refCrumbMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: window.openBranchPicker("left")
                    }
                }

                Icon {
                    visible: diffController.selectedFile.path !== undefined
                    svg: root.iconChevronRight
                    size: 10
                    color: theme.textFaint
                    opacity: 0.5
                }

                Text {
                    visible: diffController.selectedFile.path !== undefined
                    text: {
                        var p = diffController.selectedFile.path || ""
                        var parts = p.split("/")
                        return parts[parts.length - 1]
                    }
                    color: theme.textMuted
                    font.family: theme.mono
                    font.pixelSize: theme.fontCaption + 1
                }

                Badge {
                    visible: diffController.selectedFile.status !== undefined
                    text: diffController.selectedFile.status || ""
                    variant: {
                        var s = diffController.selectedFile.status
                        if (s === "A") return "success"
                        if (s === "D") return "danger"
                        if (s === "R") return "accent"
                        return "warning"
                    }
                }

                Item { Layout.fillWidth: true }

                Rectangle {
                    visible: !root.compactControls
                    implicitWidth: cmdkRow.implicitWidth + theme.sp2
                    implicitHeight: 16
                    radius: theme.radiusSm
                    color: theme.panelStrong
                    opacity: 0.5

                    Row {
                        id: cmdkRow
                        anchors.centerIn: parent
                        spacing: 2

                        Icon {
                            anchors.verticalCenter: parent.verticalCenter
                            svg: root.iconCommand
                            size: 8
                            color: theme.textFaint
                        }

                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            text: "K"
                            color: theme.textFaint
                            font.family: theme.sans
                            font.pixelSize: 8
                            font.bold: true
                        }
                    }

                    MouseArea {
                        anchors.fill: parent
                        cursorShape: Qt.PointingHandCursor
                        onClicked: window.openCommandPalette()
                    }
                }
            }
        }

        // ──────────────────────────────────────────────────
        // Row 2 — Branch comparison hero
        // ──────────────────────────────────────────────────
        Rectangle {
            Layout.fillWidth: true
            Layout.topMargin: theme.sp1
            Layout.bottomMargin: theme.sp2
            implicitHeight: 62
            color: theme.canvas
            radius: theme.radiusLg
            border.color: theme.borderSoft

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: theme.sp3
                anchors.rightMargin: theme.sp3
                spacing: 0

                // ── Left ref card ──
                Rectangle {
                    id: leftRefCard
                    Layout.fillWidth: true
                    Layout.preferredHeight: 38
                    property bool pickerOpen: refPickerDropdown.showing && refPickerDropdown.target === "left" && refPickerDropdown.anchorItem === leftRefCard
                    radius: theme.radiusMd
                    color: pickerOpen ? theme.panel : (leftCardMouse.containsMouse ? theme.panelStrong : theme.panel)
                    border.width: 1
                    border.color: pickerOpen ? theme.borderSoft : (leftCardMouse.containsMouse ? theme.borderSoft : "transparent")
                    Behavior on color {
                        enabled: !(root.Window.window && root.Window.window.commandPaletteShowing)
                        ColorAnimation { duration: 35 }
                    }

                    MouseArea {
                        id: leftCardMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: window.openBranchPicker("left", leftRefCard)
                    }

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: theme.sp3
                        anchors.rightMargin: theme.sp2
                        spacing: theme.sp2

                        Text {
                            text: "BASE"
                            color: theme.textFaint
                            font.family: theme.sans
                            font.pixelSize: 8
                            font.bold: true
                            font.letterSpacing: 0.8
                            Layout.preferredWidth: 30
                        }

                        Icon {
                            svg: root.iconGitBranch
                            size: 14
                            color: theme.accent
                        }

                        Text {
                            Layout.fillWidth: true
                            text: diffController.leftRefDisplay || "base ref"
                            color: diffController.leftRefDisplay.length > 0 ? theme.textStrong : theme.textFaint
                            font.family: theme.mono
                            font.pixelSize: theme.fontSmall + 1
                            elide: Text.ElideRight
                            verticalAlignment: Text.AlignVCenter
                        }
                    }
                }

                // ── Center connector: mode selector + swap ──
                Item {
                    Layout.preferredWidth: 88
                    Layout.fillHeight: true

                    Column {
                        anchors.centerIn: parent
                        spacing: 6

                        // Compare mode button
                        Rectangle {
                            anchors.horizontalCenter: parent.horizontalCenter
                            width: modeRow.implicitWidth + theme.sp6
                            height: 26
                            radius: theme.radiusMd
                            color: modeMouse.containsMouse ? theme.panelTint : "transparent"
                            border.color: modeMouse.containsMouse ? theme.borderSoft : "transparent"
                            Behavior on color {
                                enabled: !(root.Window.window && root.Window.window.commandPaletteShowing)
                                ColorAnimation { duration: 30 }
                            }
                            Behavior on border.color {
                                enabled: !(root.Window.window && root.Window.window.commandPaletteShowing)
                                ColorAnimation { duration: 30 }
                            }

                            Text {
                                id: modeRow
                                anchors.centerIn: parent
                                text: compareModeLabel(diffController.compareMode)
                                color: modeMouse.containsMouse ? theme.textStrong : theme.textMuted
                                font.family: theme.mono
                                font.pixelSize: theme.fontBody
                                font.bold: true
                                Behavior on color {
                                    enabled: !(root.Window.window && root.Window.window.commandPaletteShowing)
                                    ColorAnimation { duration: 30 }
                                }
                            }

                            MouseArea {
                                id: modeMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: diffController.compareMode = nextCompareMode(diffController.compareMode)
                            }
                        }

                        // Swap button
                        Rectangle {
                            anchors.horizontalCenter: parent.horizontalCenter
                            width: 40
                            height: 24
                            radius: theme.radiusMd
                            color: swapMouse.containsMouse ? theme.panelTint : "transparent"
                            border.color: swapMouse.containsMouse ? theme.borderSoft : "transparent"
                            Behavior on color {
                                enabled: !(root.Window.window && root.Window.window.commandPaletteShowing)
                                ColorAnimation { duration: 30 }
                            }
                            Behavior on border.color {
                                enabled: !(root.Window.window && root.Window.window.commandPaletteShowing)
                                ColorAnimation { duration: 30 }
                            }

                            Icon {
                                anchors.centerIn: parent
                                svg: root.iconArrowRightLeft
                                size: 16
                                color: swapMouse.containsMouse ? theme.accent : theme.textFaint
                                Behavior on color {
                                    enabled: !(root.Window.window && root.Window.window.commandPaletteShowing)
                                    ColorAnimation { duration: 35 }
                                }
                            }

                            MouseArea {
                                id: swapMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: root.swapRefs()
                            }
                        }
                    }
                }

                // ── Right ref card ──
                Rectangle {
                    id: rightRefCard
                    Layout.fillWidth: true
                    Layout.preferredHeight: 38
                    property bool pickerOpen: refPickerDropdown.showing && refPickerDropdown.target === "right" && refPickerDropdown.anchorItem === rightRefCard
                    radius: theme.radiusMd
                    color: pickerOpen ? theme.panel : (rightCardMouse.containsMouse ? theme.panelStrong : theme.panel)
                    border.width: 1
                    border.color: pickerOpen ? theme.borderSoft : (rightCardMouse.containsMouse ? theme.borderSoft : "transparent")
                    Behavior on color {
                        enabled: !(root.Window.window && root.Window.window.commandPaletteShowing)
                        ColorAnimation { duration: 35 }
                    }

                    MouseArea {
                        id: rightCardMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: window.openBranchPicker("right", rightRefCard)
                    }

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: theme.sp3
                        anchors.rightMargin: theme.sp2
                        spacing: theme.sp2

                        Text {
                            text: "HEAD"
                            color: theme.textFaint
                            font.family: theme.sans
                            font.pixelSize: 8
                            font.bold: true
                            font.letterSpacing: 0.8
                            Layout.preferredWidth: 30
                        }

                        Icon {
                            svg: root.iconGitBranch
                            size: 14
                            color: theme.accent
                        }

                        Text {
                            Layout.fillWidth: true
                            text: diffController.rightRefDisplay || "head ref"
                            color: diffController.rightRefDisplay.length > 0 ? theme.textStrong : theme.textFaint
                            font.family: theme.mono
                            font.pixelSize: theme.fontSmall + 1
                            elide: Text.ElideRight
                            verticalAlignment: Text.AlignVCenter
                        }
                    }
                }

                Item { Layout.preferredWidth: theme.sp2 }

                // ── Compare button ──
                ActionButton {
                    Layout.preferredHeight: 36
                    text: diffController.comparing ? "Comparing…" : "Compare"
                    tone: "accent"
                    active: true
                    toolTip: "Run comparison"
                    onClicked: root.runCompare()
                }
            }
        }

        // ──────────────────────────────────────────────────
        // Row 3 — View controls strip
        // ──────────────────────────────────────────────────
        Rectangle {
            Layout.fillWidth: true
            color: theme.panel
            implicitHeight: 30
            radius: theme.radiusLg

            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                height: theme.radiusLg
                color: theme.panel
            }

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: theme.sp3
                anchors.rightMargin: theme.sp3
                spacing: theme.sp2

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

                Item { Layout.fillWidth: true }

                Text {
                    visible: diffPane.surfaceItem && diffPane.surfaceItem.contentHeight > 0
                    text: {
                        var contentH = diffPane.surfaceItem ? diffPane.surfaceItem.contentHeight : 1
                        var viewportY = diffPane.surfaceItem ? diffPane.surfaceItem.viewportY : 0
                        var viewportH = diffPane.surfaceItem ? diffPane.surfaceItem.viewportHeight : 1
                        var pct = Math.min(100, Math.round((viewportY + viewportH) / contentH * 100))
                        return pct + "%"
                    }
                    color: theme.textFaint
                    font.family: theme.mono
                    font.pixelSize: theme.fontCaption
                }

                Text {
                    visible: diffController.files.length > 0
                    text: diffController.files.length + (diffController.files.length === 1 ? " file" : " files")
                    color: theme.textFaint
                    font.family: theme.mono
                    font.pixelSize: theme.fontCaption
                }
            }
        }

        Item { implicitHeight: theme.sp1 }

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
                renderKey: diffController.selectedFileRenderKey
                layoutMode: diffController.layoutMode
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
