import QtQuick
import QtQuick.Layouts
import QtQuick.Window
import "../components"

Rectangle {
    id: root

    property bool compactControls: width < 800
    property bool prExpanded: true

    signal browseRequested()
    signal pickBranchRequested(string target)

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

    function repoLabel() {
        if (diffController.repoPath.length === 0) return "No repository"
        var parts = diffController.repoPath.split("/")
        return parts.length > 0 ? parts[parts.length - 1] : diffController.repoPath
    }

    function runCompare() {
        if (leftRefField.text !== diffController.leftRefDisplay)
            diffController.leftRef = leftRefField.text
        if (rightRefField.text !== diffController.rightRefDisplay)
            diffController.rightRef = rightRefField.text
        diffController.compare()
    }

    color: theme.appBg

    Connections {
        target: diffController
        function onLeftRefChanged() { leftRefField.text = diffController.leftRefDisplay }
        function onRightRefChanged() { rightRefField.text = diffController.rightRefDisplay }
        function onOauthStateChanged() {
            if (diffController.oauthVerificationUri.length > 0) {
                Qt.openUrlExternally(diffController.oauthVerificationUri)
            }
        }
    }

    ColumnLayout {
        anchors.centerIn: parent
        spacing: theme.sp4
        width: Math.min(640, parent.width - theme.sp12)

        RowLayout {
            Layout.fillWidth: true
            spacing: theme.sp2

            Text {
                text: repoLabel()
                color: theme.textStrong
                font.family: theme.sans
                font.pixelSize: theme.fontTitle
                font.bold: true
            }

            Item { Layout.fillWidth: true }

            ActionButton {
                text: "Change"
                compact: true
                onClicked: root.browseRequested()
            }

            ActionButton {
                text: "Back"
                compact: true
                onClicked: diffController.goBack()
            }
        }

        Text {
            text: diffController.repoPath
            color: theme.textFaint
            font.family: theme.mono
            font.pixelSize: theme.fontSmall
            elide: Text.ElideMiddle
            Layout.fillWidth: true
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 1
            color: theme.borderSoft
        }

        // --- Ref inputs ---
        RowLayout {
            Layout.fillWidth: true
            spacing: theme.sp2

            Rectangle {
                id: leftRefCard
                Layout.fillWidth: true
                implicitHeight: 30
                radius: theme.radiusSm
                color: leftSetupHover.hovered ? theme.panelStrong : theme.panel
                border.width: 1
                border.color: theme.borderSoft
                Behavior on color {
                    enabled: !(Window.window && Window.window.commandPaletteShowing)
                    ColorAnimation { duration: 35 }
                }

                HoverHandler {
                    id: leftSetupHover
                }

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: theme.sp3
                    anchors.rightMargin: theme.sp2
                    spacing: theme.sp2

                    InputField {
                        id: leftRefField
                        Layout.fillWidth: true
                        monospace: true
                        borderless: true
                        compact: true
                        activeFocusOnTab: true
                        text: diffController.leftRefDisplay
                        placeholderText: "Base ref (e.g. main)"
                        onEdited: window.syncBranchPickerQuery("left", text)
                        onInputFocusChanged: function(focused) {
                            if (focused)
                                window.openBranchPicker("left", leftRefCard, text, true, leftRefCard)
                        }
                        onSubmitted: root.runCompare()
                    }

                    Icon {
                        svg: '<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>'
                        size: 14
                        color: theme.textFaint

                        MouseArea {
                            anchors.fill: parent
                            anchors.margins: -6
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: {
                                leftRefField.focusInput()
                                window.openBranchPicker("left", leftRefCard, leftRefField.text, true, leftRefCard)
                            }
                        }
                    }
                }
            }

            ActionButton {
                text: compareModeLabel(diffController.compareMode)
                compact: true
                onClicked: diffController.compareMode = nextCompareMode(diffController.compareMode)
            }

            Rectangle {
                id: rightRefCard
                Layout.fillWidth: true
                implicitHeight: 30
                radius: theme.radiusSm
                color: rightSetupHover.hovered ? theme.panelStrong : theme.panel
                border.width: 1
                border.color: theme.borderSoft
                Behavior on color {
                    enabled: !(Window.window && Window.window.commandPaletteShowing)
                    ColorAnimation { duration: 35 }
                }

                HoverHandler {
                    id: rightSetupHover
                }

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: theme.sp3
                    anchors.rightMargin: theme.sp2
                    spacing: theme.sp2

                    InputField {
                        id: rightRefField
                        Layout.fillWidth: true
                        monospace: true
                        borderless: true
                        compact: true
                        activeFocusOnTab: true
                        text: diffController.rightRefDisplay
                        placeholderText: "Head ref (e.g. feature)"
                        onEdited: window.syncBranchPickerQuery("right", text)
                        onInputFocusChanged: function(focused) {
                            if (focused)
                                window.openBranchPicker("right", rightRefCard, text, true, rightRefCard)
                        }
                        onSubmitted: root.runCompare()
                    }

                    Icon {
                        svg: '<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>'
                        size: 14
                        color: theme.textFaint

                        MouseArea {
                            anchors.fill: parent
                            anchors.margins: -6
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: {
                                rightRefField.focusInput()
                                window.openBranchPicker("right", rightRefCard, rightRefField.text, true, rightRefCard)
                            }
                        }
                    }
                }
            }
        }

        // --- Compare button row (no renderer/layout — those are on diff view) ---
        RowLayout {
            Layout.fillWidth: true
            spacing: theme.sp2

            Item { Layout.fillWidth: true }

            ActionButton {
                text: diffController.comparing ? "Comparing…" : "Compare"
                tone: "accent"
                active: true
                onClicked: root.runCompare()
            }
        }

        // --- Comparing indicator ---
        Text {
            visible: diffController.comparing
            text: "Comparing…"
            color: theme.textFaint
            font.family: theme.sans
            font.pixelSize: theme.fontBody
            Layout.alignment: Qt.AlignHCenter
        }

        // --- Error banner ---
        Rectangle {
            visible: diffController.errorMessage.length > 0
            Layout.fillWidth: true
            implicitHeight: errorText.implicitHeight + theme.sp4
            radius: theme.radiusMd
            color: theme.dangerBg
            border.color: theme.dangerBorder

            Text {
                id: errorText
                anchors.fill: parent
                anchors.margins: theme.sp2
                text: diffController.errorMessage
                color: theme.dangerText
                font.family: theme.sans
                font.pixelSize: theme.fontBody
                wrapMode: Text.Wrap
            }
        }

        // --- GitHub Pull Request section ---
        Rectangle {
            Layout.fillWidth: true
            Layout.topMargin: theme.sp1
            implicitHeight: prSection.implicitHeight + theme.sp4
            radius: theme.radiusMd
            color: theme.panel
            border.color: theme.borderSoft

            ColumnLayout {
                id: prSection
                anchors.fill: parent
                anchors.margins: theme.sp3
                spacing: theme.sp2

                // Header
                RowLayout {
                    Layout.fillWidth: true
                    spacing: theme.sp2

                    Text {
                        text: (root.prExpanded ? "▾ " : "▸ ") + "GitHub Pull Request"
                        color: theme.textStrong
                        font.family: theme.sans
                        font.pixelSize: theme.fontBody
                        font.bold: true

                        MouseArea {
                            anchors.fill: parent
                            cursorShape: Qt.PointingHandCursor
                            onClicked: root.prExpanded = !root.prExpanded
                        }
                    }

                    Item { Layout.fillWidth: true }

                    // Authenticated badge + logout
                    RowLayout {
                        visible: diffController.hasGithubToken
                        spacing: theme.sp1

                        Rectangle {
                            implicitWidth: tokenOkLabel.implicitWidth + theme.sp4
                            implicitHeight: 18
                            radius: theme.radiusSm
                            color: theme.successBg
                            border.color: theme.successBorder

                            Text {
                                id: tokenOkLabel
                                anchors.centerIn: parent
                                text: "Authenticated"
                                color: theme.successText
                                font.family: theme.sans
                                font.pixelSize: 8
                                font.bold: true
                            }
                        }

                        ActionButton {
                            text: "Logout"
                            compact: true
                            onClicked: diffController.githubToken = ""
                        }
                    }
                }

                // --- Not authenticated: login options ---
                ColumnLayout {
                    visible: root.prExpanded && !diffController.hasGithubToken && !diffController.oauthInProgress
                    Layout.fillWidth: true
                    spacing: theme.sp3

                    ActionButton {
                        text: "Login with GitHub"
                        tone: "accent"
                        active: true
                        onClicked: diffController.startOAuthLogin()
                    }

                    Rectangle {
                        Layout.fillWidth: true
                        implicitHeight: 1
                        color: theme.borderSoft

                        Text {
                            anchors.centerIn: parent
                            text: "  or  "
                            color: theme.textFaint
                            font.family: theme.sans
                            font.pixelSize: theme.fontCaption

                            Rectangle {
                                anchors.fill: parent
                                color: theme.panel
                                z: -1
                            }
                        }
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: theme.sp2

                        InputField {
                            id: tokenField
                            Layout.fillWidth: true
                            monospace: true
                            placeholderText: "Paste a personal access token"
                            onSubmitted: {
                                if (text.length > 0) diffController.githubToken = text
                            }
                        }

                        ActionButton {
                            text: "Save"
                            compact: true
                            tone: "success"
                            onClicked: {
                                if (tokenField.text.length > 0) {
                                    diffController.githubToken = tokenField.text
                                }
                            }
                        }
                    }

                    Text {
                        text: "Token needs <b>repo</b> scope. Create one at github.com/settings/tokens"
                        color: theme.textFaint
                        font.family: theme.sans
                        font.pixelSize: theme.fontCaption
                        textFormat: Text.RichText
                        Layout.fillWidth: true
                    }
                }

                // --- OAuth in progress ---
                ColumnLayout {
                    visible: root.prExpanded && diffController.oauthInProgress
                    Layout.fillWidth: true
                    spacing: theme.sp3

                    Text {
                        text: "A browser tab has been opened. Enter the code:"
                        color: theme.textMuted
                        font.family: theme.sans
                        font.pixelSize: theme.fontBody
                        Layout.alignment: Qt.AlignHCenter
                    }

                    Text {
                        text: diffController.oauthUserCode
                        color: theme.textStrong
                        font.family: theme.mono
                        font.pixelSize: theme.fontHeading
                        font.bold: true
                        Layout.alignment: Qt.AlignHCenter
                    }

                    RowLayout {
                        Layout.alignment: Qt.AlignHCenter
                        spacing: theme.sp2

                        Text {
                            text: "Waiting for authorization…"
                            color: theme.textFaint
                            font.family: theme.sans
                            font.pixelSize: theme.fontSmall
                        }

                        ActionButton {
                            text: "Cancel"
                            compact: true
                            onClicked: diffController.cancelOAuthLogin()
                        }
                    }
                }

                // --- Authenticated: PR URL input ---
                RowLayout {
                    visible: root.prExpanded && diffController.hasGithubToken
                    Layout.fillWidth: true
                    spacing: theme.sp2

                    InputField {
                        id: prUrlField
                        Layout.fillWidth: true
                        monospace: true
                        placeholderText: "https://github.com/owner/repo/pull/123"
                        onSubmitted: {
                            if (text.length > 0) diffController.openPullRequest(text)
                        }
                    }

                    ActionButton {
                        text: diffController.pullRequestLoading ? "Loading…" : "Open PR"
                        compact: true
                        tone: "accent"
                        active: true
                        onClicked: {
                            if (prUrlField.text.length > 0) {
                                diffController.openPullRequest(prUrlField.text)
                            }
                        }
                    }
                }

                // --- PR info card ---
                Rectangle {
                    visible: root.prExpanded && diffController.pullRequestInfo.title !== undefined
                    Layout.fillWidth: true
                    implicitHeight: prInfoCol.implicitHeight + theme.sp3
                    radius: theme.radiusSm
                    color: theme.panelStrong

                    ColumnLayout {
                        id: prInfoCol
                        anchors.fill: parent
                        anchors.margins: theme.sp2
                        spacing: theme.sp1

                        Text {
                            Layout.fillWidth: true
                            text: diffController.pullRequestInfo.title || ""
                            color: theme.textStrong
                            font.family: theme.sans
                            font.pixelSize: theme.fontBody
                            font.bold: true
                            wrapMode: Text.Wrap
                        }

                        RowLayout {
                            spacing: theme.sp2

                            Text {
                                text: "#" + (diffController.pullRequestInfo.number || "")
                                color: theme.textFaint
                                font.family: theme.mono
                                font.pixelSize: theme.fontCaption
                            }

                            Text {
                                text: (diffController.pullRequestInfo.author || "")
                                color: theme.textMuted
                                font.family: theme.sans
                                font.pixelSize: theme.fontCaption
                            }

                            Text {
                                text: (diffController.pullRequestInfo.baseBranch || "") + " ← " + (diffController.pullRequestInfo.headBranch || "")
                                color: theme.accentStrong
                                font.family: theme.mono
                                font.pixelSize: theme.fontCaption
                            }

                            Text {
                                text: "+" + (diffController.pullRequestInfo.additions || 0)
                                color: theme.successText
                                font.family: theme.mono
                                font.pixelSize: theme.fontCaption
                            }

                            Text {
                                text: "-" + (diffController.pullRequestInfo.deletions || 0)
                                color: theme.dangerText
                                font.family: theme.mono
                                font.pixelSize: theme.fontCaption
                            }
                        }
                    }
                }
            }
        }
    }

}
