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
    title: "diffy"
    color: theme.appBg

    property string previousView: "welcome"

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

        NumberAnimation on opacity { duration: 180; easing.type: Easing.InOutQuad }
        NumberAnimation { id: slideAnim; target: welcomeView; property: "slideX"; duration: 220; easing.type: Easing.OutCubic }

        onOpenRepositoryRequested: diffController.openRepositoryPicker()
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

        NumberAnimation on opacity { duration: 180; easing.type: Easing.InOutQuad }
        NumberAnimation { id: compSlideAnim; target: compareView; property: "slideX"; duration: 220; easing.type: Easing.OutCubic }

        onBrowseRequested: diffController.openRepositoryPicker()
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

        NumberAnimation on opacity { duration: 180; easing.type: Easing.InOutQuad }
        NumberAnimation { id: diffSlideAnim; target: diffView; property: "slideX"; duration: 220; easing.type: Easing.OutCubic }
    }

    RepositoryPickerOverlay {
        anchors.fill: parent
    }

    ShortcutOverlay {
        id: shortcutOverlay
    }

    CommandPalette {
        id: commandPalette
        onActionTriggered: function(item) {
            if (item.type === "file") {
                diffController.selectFile(item.index)
            } else if (item.type === "branch") {
                if (window.branchPickTarget === "right") {
                    diffController.rightRef = item.value
                } else {
                    diffController.leftRef = item.value
                }
                window.branchPickTarget = ""
            } else if (item.type === "action") {
                if (item.value === "compare") diffController.compare()
                else if (item.value === "back") diffController.goBack()
                else if (item.value === "openRepo") diffController.openRepositoryPicker()
                else if (item.value === "shortcuts") shortcutOverlay.showing = true
            }
        }
    }

    property string branchPickTarget: ""

    function openBranchPicker(target) {
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

    function openCommandPalette() {
        var items = []

        // Actions
        items.push({label: "Compare", detail: "", category: "Action", type: "action", value: "compare"})
        items.push({label: "Go Back", detail: "Alt+←", category: "Action", type: "action", value: "back"})
        items.push({label: "Open Repository", detail: "", category: "Action", type: "action", value: "openRepo"})
        items.push({label: "Keyboard Shortcuts", detail: "?", category: "Action", type: "action", value: "shortcuts"})

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
            if (commandPalette.showing) {
                commandPalette.close()
            } else if (shortcutOverlay.showing) {
                shortcutOverlay.showing = false
            } else if (diffController.repositoryPickerVisible) {
                diffController.closeRepositoryPicker()
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
}
