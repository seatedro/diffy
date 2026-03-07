import QtQuick
import QtQuick.Window
import "components"
import "views"

Window {
    id: window
    visible: true
    width: 1540
    height: 940
    minimumWidth: 960
    minimumHeight: 720
    title: "diffy"
    color: theme.appBg

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
        Behavior on opacity { NumberAnimation { duration: 180; easing.type: Easing.InOutQuad } }
        onOpenRepositoryRequested: diffController.openRepositoryPicker()
        onOpenRecentRequested: function(path) { diffController.openRepository(path) }
    }

    CompareSetupView {
        id: compareView
        anchors.fill: parent
        visible: opacity > 0
        opacity: diffController.currentView === "compare" ? 1.0 : 0.0
        Behavior on opacity { NumberAnimation { duration: 180; easing.type: Easing.InOutQuad } }
        onBrowseRequested: diffController.openRepositoryPicker()
    }

    DiffBrowsingView {
        id: diffView
        anchors.fill: parent
        visible: opacity > 0
        opacity: diffController.currentView === "diff" ? 1.0 : 0.0
        Behavior on opacity { NumberAnimation { duration: 180; easing.type: Easing.InOutQuad } }
    }

    RepositoryPickerOverlay {
        anchors.fill: parent
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
            if (diffController.repositoryPickerVisible) {
                diffController.closeRepositoryPicker()
            } else {
                diffController.goBack()
            }
        }
    }
}
