import QtQuick
import QtQuick.Layouts
import "../components"

Rectangle {
    id: root

    property bool compactControls: width < 800
    property string pickerTarget: ""
    property bool prExpanded: true

    signal browseRequested()

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
        diffController.leftRef = leftRefField.text
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

            InputField {
                id: leftRefField
                Layout.fillWidth: true
                monospace: true
                text: diffController.leftRefDisplay
                placeholderText: "Base ref (e.g. main)"
                onSubmitted: root.runCompare()
            }

            ActionButton {
                text: "Branch"
                compact: true
                onClicked: {
                    root.pickerTarget = "left"
                    branchPicker.model = diffController.branches
                    branchPicker.showing = true
                }
            }

            ActionButton {
                text: compareModeLabel(diffController.compareMode)
                compact: true
                onClicked: diffController.compareMode = nextCompareMode(diffController.compareMode)
            }

            InputField {
                id: rightRefField
                Layout.fillWidth: true
                monospace: true
                text: diffController.rightRefDisplay
                placeholderText: "Head ref (e.g. feature)"
                onSubmitted: root.runCompare()
            }

            ActionButton {
                text: "Branch"
                compact: true
                onClicked: {
                    root.pickerTarget = "right"
                    branchPicker.model = diffController.branches
                    branchPicker.showing = true
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

    PickerOverlay {
        id: branchPicker
        anchors.fill: parent
        title: "Select Branch"
        displayRole: "name"
        badgeRole: "isHead"
        onItemSelected: function(index, item) {
            if (root.pickerTarget === "left") {
                diffController.leftRef = item.name
            } else {
                diffController.rightRef = item.name
            }
            branchPicker.showing = false
        }
        onDismissed: branchPicker.showing = false
    }
}
