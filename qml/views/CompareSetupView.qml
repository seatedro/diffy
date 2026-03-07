import QtQuick
import QtQuick.Layouts
import "../components"

Rectangle {
    id: root

    property bool compactControls: width < 800
    property string pickerTarget: ""

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
        diffController.leftRef = leftRefField.text
        diffController.rightRef = rightRefField.text
        diffController.compare()
    }

    color: theme.appBg

    Connections {
        target: diffController
        function onLeftRefChanged() { leftRefField.text = diffController.leftRefDisplay }
        function onRightRefChanged() { rightRefField.text = diffController.rightRefDisplay }
    }

    ColumnLayout {
        anchors.centerIn: parent
        spacing: 20
        width: Math.min(640, parent.width - 48)

        RowLayout {
            Layout.fillWidth: true
            spacing: 8

            Text {
                text: repoLabel()
                color: theme.textStrong
                font.family: theme.sans
                font.pixelSize: 18
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
            font.pixelSize: 10
            elide: Text.ElideMiddle
            Layout.fillWidth: true
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 1
            color: theme.borderSoft
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: 8

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

        RowLayout {
            Layout.fillWidth: true
            spacing: 8

            ActionButton {
                text: diffController.renderer === "difftastic" ? "Difftastic" : "Built-in"
                compact: true
                tone: diffController.renderer === "difftastic" ? "accent" : "neutral"
                active: diffController.renderer === "difftastic"
                onClicked: diffController.renderer = nextRenderer(diffController.renderer)
            }

            ActionButton {
                text: diffController.layoutMode === "split" ? "Split" : "Unified"
                compact: true
                active: diffController.layoutMode === "split"
                onClicked: diffController.layoutMode = diffController.layoutMode === "unified" ? "split" : "unified"
            }

            Item { Layout.fillWidth: true }

            ActionButton {
                text: "Compare"
                tone: "accent"
                active: true
                onClicked: root.runCompare()
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
            Layout.topMargin: 4
            implicitHeight: prSection.implicitHeight + 20
            radius: 6
            color: theme.panel
            border.color: theme.borderSoft

            ColumnLayout {
                id: prSection
                anchors.fill: parent
                anchors.margins: 10
                spacing: 8

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 8

                    Text {
                        text: "GitHub Pull Request"
                        color: theme.textStrong
                        font.family: theme.sans
                        font.pixelSize: 11
                        font.bold: true
                    }

                    Item { Layout.fillWidth: true }

                    Rectangle {
                        visible: diffController.hasGithubToken
                        implicitWidth: tokenOkLabel.implicitWidth + 14
                        implicitHeight: 18
                        radius: 4
                        color: theme.successBg
                        border.color: theme.successBorder

                        Text {
                            id: tokenOkLabel
                            anchors.centerIn: parent
                            text: "Token set"
                            color: theme.successText
                            font.family: theme.sans
                            font.pixelSize: 8
                            font.bold: true
                        }
                    }
                }

                RowLayout {
                    visible: !diffController.hasGithubToken
                    Layout.fillWidth: true
                    spacing: 8

                    InputField {
                        id: tokenField
                        Layout.fillWidth: true
                        monospace: true
                        placeholderText: "GitHub personal access token"
                        onSubmitted: diffController.githubToken = text
                    }

                    ActionButton {
                        text: "Save Token"
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
                    visible: !diffController.hasGithubToken
                    text: "A token is required for private repos. Create one at github.com/settings/tokens with repo scope."
                    color: theme.textFaint
                    font.family: theme.sans
                    font.pixelSize: 9
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                }

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 8

                    InputField {
                        id: prUrlField
                        Layout.fillWidth: true
                        monospace: true
                        placeholderText: "https://github.com/owner/repo/pull/123"
                        onSubmitted: {
                            if (diffController.hasGithubToken) diffController.openPullRequest(text)
                        }
                    }

                    ActionButton {
                        text: diffController.pullRequestLoading ? "Loading…" : "Open PR"
                        compact: true
                        tone: diffController.hasGithubToken ? "accent" : "neutral"
                        active: diffController.hasGithubToken
                        onClicked: {
                            if (prUrlField.text.length > 0 && diffController.hasGithubToken) {
                                diffController.openPullRequest(prUrlField.text)
                            }
                        }
                    }
                }

                Rectangle {
                    visible: diffController.pullRequestInfo.title !== undefined
                    Layout.fillWidth: true
                    implicitHeight: prInfoCol.implicitHeight + 12
                    radius: 4
                    color: theme.panelStrong

                    ColumnLayout {
                        id: prInfoCol
                        anchors.fill: parent
                        anchors.margins: 8
                        spacing: 4

                        Text {
                            Layout.fillWidth: true
                            text: diffController.pullRequestInfo.title || ""
                            color: theme.textStrong
                            font.family: theme.sans
                            font.pixelSize: 11
                            font.bold: true
                            wrapMode: Text.Wrap
                        }

                        RowLayout {
                            spacing: 8

                            Text {
                                text: "#" + (diffController.pullRequestInfo.number || "")
                                color: theme.textFaint
                                font.family: theme.mono
                                font.pixelSize: 9
                            }

                            Text {
                                text: (diffController.pullRequestInfo.author || "")
                                color: theme.textMuted
                                font.family: theme.sans
                                font.pixelSize: 9
                            }

                            Text {
                                text: (diffController.pullRequestInfo.baseBranch || "") + " ← " + (diffController.pullRequestInfo.headBranch || "")
                                color: theme.accentStrong
                                font.family: theme.mono
                                font.pixelSize: 9
                            }

                            Text {
                                text: "+" + (diffController.pullRequestInfo.additions || 0)
                                color: theme.successText
                                font.family: theme.mono
                                font.pixelSize: 9
                            }

                            Text {
                                text: "-" + (diffController.pullRequestInfo.deletions || 0)
                                color: theme.dangerText
                                font.family: theme.mono
                                font.pixelSize: 9
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
