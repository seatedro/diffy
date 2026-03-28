import QtQuick
import QtQuick.Window
import "components"
import "views"

Window {
    id: window
    visible: true
    width: 1540
    height: 940
    minimumWidth: 480
    minimumHeight: 400
    title: {
        var parts = ["diffy"]
        if (diffController.repoPath.length > 0) {
            var repoParts = diffController.repoPath.split("/")
            parts.push(repoParts[repoParts.length - 1])
        }
        if (diffController.currentView === "diff" && diffController.leftRefDisplay.length > 0) {
            var compareModeSeparator = ".."
            if (diffController.compareMode === "three-dot")
                compareModeSeparator = "..."
            else if (diffController.compareMode === "single-commit")
                compareModeSeparator = "@"
            parts.push(diffController.leftRefDisplay + compareModeSeparator + diffController.rightRefDisplay)
        }
        if (diffController.selectedFile && diffController.selectedFile.path) {
            var fileParts = diffController.selectedFile.path.split("/")
            parts.push(fileParts[fileParts.length - 1])
        }
        return parts.join(" \u2014 ")
    }
    color: theme.appBg

    property string previousView: "welcome"
    property bool commandPaletteShowing: commandPalette.showing

    function viewIndex(name) {
        if (name === "welcome") return 0
        if (name === "compare") return 1
        return 2
    }

    Connections {
        target: diffController
        function onCurrentViewChanged() {
            var oldIdx = viewIndex(window.previousView)
            var newIdx = viewIndex(diffController.currentView)
            var goingForward = newIdx > oldIdx

            // Slide outgoing view
            var outgoing = oldIdx === 0 ? welcomeView : (oldIdx === 1 ? compareView : diffView)
            var incoming = newIdx === 0 ? welcomeView : (newIdx === 1 ? compareView : diffView)

            outgoing.slideOut(goingForward)
            incoming.slideIn(goingForward)

            window.previousView = diffController.currentView
        }
    }

    Rectangle {
        anchors.fill: parent
        color: theme.appBg
    }

    // Global progress bar at top of window
    ProgressBar {
        id: globalProgress
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        z: 100
        active: diffController.comparing || diffController.pullRequestLoading
    }

    WelcomeView {
        id: welcomeView
        anchors.fill: parent
        visible: opacity > 0
        opacity: diffController.currentView === "welcome" ? 1.0 : 0.0

        property real slideX: 0
        transform: Translate { x: welcomeView.slideX }

        function slideIn(forward) {
            slideX = forward ? -100 : 100
            slideAnim.to = 0
            slideAnim.start()
        }
        function slideOut(forward) {
            slideAnim.to = forward ? -100 : 100
            slideAnim.start()
        }

        NumberAnimation on opacity { duration: 70; easing.type: Easing.InOutQuad }
        NumberAnimation { id: slideAnim; target: welcomeView; property: "slideX"; duration: 80; easing.type: Easing.OutCubic }

        onOpenRepositoryRequested: window.openRepoPicker()
        onOpenRecentRequested: function(path) { diffController.openRepository(path) }
    }

    CompareSetupView {
        id: compareView
        anchors.fill: parent
        visible: opacity > 0
        opacity: diffController.currentView === "compare" ? 1.0 : 0.0

        property real slideX: 0
        transform: Translate { x: compareView.slideX }

        function slideIn(forward) {
            slideX = forward ? 100 : -100
            compSlideAnim.to = 0
            compSlideAnim.start()
        }
        function slideOut(forward) {
            compSlideAnim.to = forward ? -100 : 100
            compSlideAnim.start()
        }

        NumberAnimation on opacity { duration: 70; easing.type: Easing.InOutQuad }
        NumberAnimation { id: compSlideAnim; target: compareView; property: "slideX"; duration: 80; easing.type: Easing.OutCubic }

        onBrowseRequested: window.openRepoPicker()
        onPickBranchRequested: function(target) { window.openBranchPicker(target) }
    }

    DiffBrowsingView {
        id: diffView
        anchors.fill: parent
        visible: opacity > 0
        opacity: diffController.currentView === "diff" ? 1.0 : 0.0

        property real slideX: 0
        transform: Translate { x: diffView.slideX }

        function slideIn(forward) {
            slideX = forward ? 100 : -100
            diffSlideAnim.to = 0
            diffSlideAnim.start()
        }
        function slideOut(forward) {
            diffSlideAnim.to = forward ? -100 : 100
            diffSlideAnim.start()
        }

        NumberAnimation on opacity { duration: 70; easing.type: Easing.InOutQuad }
        NumberAnimation { id: diffSlideAnim; target: diffView; property: "slideX"; duration: 80; easing.type: Easing.OutCubic }
    }

    ShortcutOverlay {
        id: shortcutOverlay
    }

    DebugOverlay {
        id: debugOverlay
        surface: diffView.visible ? diffView.surfaceItem : null
    }

    Shortcut {
        sequence: "`"
        onActivated: debugOverlay.showing = !debugOverlay.showing
    }

    CommandPalette {
        id: commandPalette
        onActionTriggered: function(item) {
            if (item.type === "action" && item.value === "submenu:root") {
                window.openCommandPaletteRoot()
                return
            }
            if (item.type === "action" && item.value === "submenu:theme") {
                window.openThemePicker()
                return
            }
            if (item.type === "action" && item.value === "submenu:appearance") {
                window.openAppearancePicker()
                return
            }

            if (item.type === "file") {
                diffController.selectFile(item.index)
            } else if (item.type === "branch") {
                if (window.branchPickTarget === "right") {
                    diffController.rightRef = item.value
                } else {
                    diffController.leftRef = item.value
                }
                window.branchPickTarget = ""
            } else if (item.type === "repo") {
                diffController.openRepository(item.value)
            } else if (item.type === "action") {
                if (item.value === "compare") diffController.compare()
                else if (item.value === "back") diffController.goBack()
                else if (item.value === "openRepo") window.openRepoPicker()
                else if (item.value === "browseRepo") diffController.openRepositoryFromDialog()
                else if (item.value === "shortcuts") shortcutOverlay.showing = true
                else if (item.value && item.value.startsWith("theme:")) {
                    theme.setTheme(item.value.substring(6), true)
                    window.themePreviewActive = false
                } else if (item.value && item.value.startsWith("mode:")) {
                    theme.setMode(item.value.substring(5), true)
                    window.themePreviewActive = false
                }
            }
        }
        onItemHighlighted: function(item) {
            if (!window.themePreviewActive) return
            if (!item || !item.value) {
                theme.setTheme(window.previewTheme, false)
                theme.setMode(window.previewMode, false)
                return
            }
            if (window.commandPaletteLevel === "theme" && item.value.startsWith("theme:")) {
                theme.setTheme(item.value.substring(6), false)
            } else if (window.commandPaletteLevel === "appearance" && item.value.startsWith("mode:")) {
                theme.setMode(item.value.substring(5), false)
            } else {
                theme.setTheme(window.previewTheme, false)
                theme.setMode(window.previewMode, false)
            }
        }
        onClosed: {
            window.restoreThemePreview()
            window.commandPaletteLevel = "root"
        }
    }

    property var repoPickerEntries: []

    function rebuildRepoPickerEntries() {
        var m = diffController.repositoryPickerModel
        var arr = []
        var count = m.entryCount()
        for (var i = 0; i < count; i++) {
            arr.push({
                display: m.entryName(i),
                path: m.entryPathAt(i),
                badgeText: m.entryIsGitRepo(i) ? "repo" : "",
                originalIndex: i
            })
        }
        repoPickerEntries = arr
    }

    Connections {
        target: diffController.repositoryPickerModel
        function onCurrentPathChanged() {
            window.rebuildRepoPickerEntries()
        }
    }

    PickerOverlay {
        id: repoPickerOverlay
        anchors.fill: parent
        z: 260
        showing: diffController.repositoryPickerVisible
        title: "Open Repository"
        displayRole: "display"
        badgeRole: "badgeText"
        model: window.repoPickerEntries
        breadcrumb: diffController.repositoryPickerModel.currentPath
        showUpButton: true
        pinnedItem: diffController.repositoryPickerModel.currentPathIsRepository
            ? { display: "Open this directory", badge: "repo" } : null

        onShowingChanged: {
            if (showing) window.rebuildRepoPickerEntries()
        }
        onNavigateUp: diffController.navigateRepositoryPickerUp()
        onPinnedItemActivated: diffController.openCurrentRepositoryFromPicker()
        onItemSelected: function(index, item) {
            diffController.activateRepositoryPickerEntry(item.originalIndex)
        }
        onDismissed: diffController.closeRepositoryPicker()
    }

    property string branchPickTarget: ""
    property string commandPaletteLevel: "root"
    property bool themePreviewActive: false
    property string previewTheme: ""
    property string previewMode: ""

    RefPickerDropdown {
        id: refPickerDropdown
        onRefSelected: function(target, value) {
            if (target === "right") {
                diffController.rightRef = value
            } else {
                diffController.leftRef = value
            }
        }
    }

    function openRefPicker(target, anchorElement, initialQuery, useExternalInput, passthroughElement) {
        refPickerDropdown.open(target, anchorElement, initialQuery, useExternalInput, passthroughElement)
    }

    function syncBranchPickerQuery(target, query) {
        refPickerDropdown.syncQuery(target, query)
    }

    function openRepoPicker() {
        var items = []
        items.push({label: "Browse filesystem…", detail: "", category: "Action", type: "action", value: "browseRepo"})
        var recents = diffController.recentRepositories
        for (var i = 0; i < recents.length; ++i) {
            var parts = recents[i].split("/")
            items.push({
                label: parts[parts.length - 1],
                detail: recents[i],
                category: "Recent",
                type: "repo",
                value: recents[i]
            })
        }
        commandPalette.open(items)
    }

    function openBranchPicker(target, anchorElement, initialQuery, useExternalInput, passthroughElement) {
        if (anchorElement) {
            openRefPicker(target, anchorElement, initialQuery, useExternalInput, passthroughElement)
        } else {
            branchPickTarget = target
            var items = []
            var branches = diffController.branches
            for (var i = 0; i < branches.length; ++i) {
                items.push({
                    label: branches[i].name,
                    detail: branches[i].isHead ? "HEAD" : (branches[i].isRemote ? "remote" : ""),
                    category: "Branch",
                    type: "branch",
                    value: branches[i].name
                })
            }
            commandPalette.open(items)
        }
    }

    function restoreThemePreview() {
        if (!themePreviewActive) return
        theme.setTheme(previewTheme, false)
        theme.setMode(previewMode, false)
        themePreviewActive = false
    }

    function startThemePreview() {
        previewTheme = theme.currentTheme
        previewMode = theme.currentMode
        themePreviewActive = true
    }

    function openThemePicker() {
        restoreThemePreview()
        startThemePreview()
        commandPaletteLevel = "theme"
        var items = []
        items.push({label: "← Back", detail: "", category: "Navigation", type: "action", value: "submenu:root", keepOpen: true})
        var themes = theme.availableThemes
        for (var t = 0; t < themes.length; ++t) {
            items.push({
                label: themes[t],
                detail: theme.currentTheme === themes[t] ? "active" : "",
                category: "Theme",
                type: "action",
                value: "theme:" + themes[t]
            })
        }
        commandPalette.open(items)
    }

    function openAppearancePicker() {
        restoreThemePreview()
        startThemePreview()
        commandPaletteLevel = "appearance"
        var items = []
        items.push({label: "← Back", detail: "", category: "Navigation", type: "action", value: "submenu:root", keepOpen: true})
        var modes = theme.availableModes
        for (var m = 0; m < modes.length; ++m) {
            items.push({
                label: modes[m],
                detail: theme.currentMode === modes[m] ? "active" : "",
                category: "Appearance",
                type: "action",
                value: "mode:" + modes[m]
            })
        }
        commandPalette.open(items)
    }

    function openCommandPaletteRoot() {
        restoreThemePreview()
        commandPaletteLevel = "root"
        var items = []

        // Actions
        items.push({label: "Compare", detail: "", category: "Action", type: "action", value: "compare"})
        items.push({label: "Go Back", detail: "Alt+←", category: "Action", type: "action", value: "back"})
        items.push({label: "Open Repository", detail: "", category: "Action", type: "action", value: "openRepo"})
        items.push({label: "Keyboard Shortcuts", detail: "?", category: "Action", type: "action", value: "shortcuts"})
        items.push({label: "Change theme", detail: theme.currentTheme, category: "Theme", type: "action", value: "submenu:theme", keepOpen: true})
        items.push({label: "Appearance", detail: theme.currentMode, category: "Theme", type: "action", value: "submenu:appearance", keepOpen: true})

        // Changed files
        var files = diffController.files
        for (var i = 0; i < files.length; ++i) {
            items.push({
                label: files[i].path,
                detail: "+" + files[i].additions + " -" + files[i].deletions,
                category: "File",
                type: "file",
                index: i
            })
        }

        // Branches
        var branches = diffController.branches
        for (var j = 0; j < branches.length; ++j) {
            items.push({
                label: branches[j].name,
                detail: branches[j].isHead ? "HEAD" : "",
                category: "Branch",
                type: "branch",
                value: branches[j].name
            })
        }

        commandPalette.open(items)
    }

    function openCommandPalette() {
        openCommandPaletteRoot()
    }

    // Global toast
    Toast {
        id: globalToast
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.bottom: parent.bottom
        anchors.bottomMargin: theme.sp8
        z: 200
    }

    Shortcut {
        sequence: "Escape"
        onActivated: {
            if (refPickerDropdown.showing) {
                refPickerDropdown.close()
            } else if (commandPalette.showing) {
                commandPalette.close()
            } else if (shortcutOverlay.showing) {
                shortcutOverlay.showing = false
            }
        }
    }

    Shortcut {
        sequence: "Alt+Left"
        onActivated: diffController.goBack()
    }

    Shortcut {
        sequence: "Shift+/"
        onActivated: shortcutOverlay.showing = !shortcutOverlay.showing
    }

    Shortcut {
        sequence: "Ctrl+K"
        onActivated: window.openCommandPalette()
    }

    Shortcut {
        sequence: "Ctrl+P"
        onActivated: window.openCommandPalette()
    }

    Shortcut {
        sequence: "Ctrl+Shift+T"
        onActivated: {
            var themes = theme.availableThemes
            var idx = themes.indexOf(theme.currentTheme)
            var next = (idx + 1) % themes.length
            theme.setTheme(themes[next])
            globalToast.show("Theme: " + themes[next] + " (" + theme.currentMode + ")", "neutral", 1500)
        }
    }

    Shortcut {
        sequence: "Ctrl+\\"
        onActivated: diffController.layoutMode = diffController.layoutMode === "unified" ? "split" : "unified"
    }

    Shortcut {
        sequence: "Ctrl+Shift+W"
        onActivated: diffController.wrapEnabled = !diffController.wrapEnabled
    }

    Shortcut {
        sequence: "Ctrl+B"
        onActivated: {
            if (diffView.visible) diffView.toggleSidebar()
        }
    }

    Shortcut {
        sequence: "Ctrl+Shift+C"
        onActivated: {
            var path = diffController.selectedFile ? diffController.selectedFile.path : ""
            if (path.length > 0) {
                diffController.copyToClipboard(path)
                globalToast.show("Copied: " + path, "success", 2000)
            }
        }
    }
}
