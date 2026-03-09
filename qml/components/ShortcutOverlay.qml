import QtQuick
import QtQuick.Layouts

Rectangle {
    id: root

    property bool showing: false

    visible: showing
    anchors.fill: parent
    color: "#99000000"
    z: 300

    MouseArea {
        anchors.fill: parent
        onClicked: root.showing = false
    }

    Rectangle {
        anchors.centerIn: parent
        width: 420
        height: col.implicitHeight + theme.sp8
        radius: theme.radiusXl
        color: theme.panel
        border.color: theme.borderSoft
        scale: root.showing ? 1.0 : 0.95
        opacity: root.showing ? 1.0 : 0

        Behavior on scale {
            SpringAnimation { spring: 10; damping: 0.85 }
        }
        Behavior on opacity {
            NumberAnimation { duration: 50; easing.type: Easing.OutCubic }
        }

        Rectangle {
            anchors.fill: parent
            anchors.topMargin: 4
            anchors.bottomMargin: -4
            radius: parent.radius
            color: theme.shadowLg
            z: -1
        }

        ColumnLayout {
            id: col
            anchors.fill: parent
            anchors.margins: theme.sp6
            spacing: theme.sp4

            Text {
                text: "Keyboard Shortcuts"
                color: theme.textStrong
                font.family: theme.sans
                font.pixelSize: theme.fontTitle
                font.bold: true
            }

            Rectangle {
                Layout.fillWidth: true
                implicitHeight: 1
                color: theme.borderSoft
            }

            // Navigation
            Text {
                text: "Navigation"
                color: theme.textFaint
                font.family: theme.sans
                font.pixelSize: theme.fontSmall
                font.bold: true
            }

            ShortcutRow { key: "j / k"; desc: "Next / previous file" }
            ShortcutRow { key: "n / N"; desc: "Next / previous hunk" }
            ShortcutRow { key: "Alt+←"; desc: "Go back" }
            ShortcutRow { key: "Ctrl+K"; desc: "Command palette" }
            ShortcutRow { key: "Ctrl+\\"; desc: "Toggle unified / split" }
            ShortcutRow { key: "Ctrl+Shift+W"; desc: "Toggle word wrap" }
            ShortcutRow { key: "Ctrl+B"; desc: "Toggle file sidebar" }
            ShortcutRow { key: "Escape"; desc: "Close overlay / picker" }

            // Scrolling
            Text {
                Layout.topMargin: theme.sp2
                text: "Scrolling"
                color: theme.textFaint
                font.family: theme.sans
                font.pixelSize: theme.fontSmall
                font.bold: true
            }

            ShortcutRow { key: "Space"; desc: "Page down" }
            ShortcutRow { key: "Shift+Space"; desc: "Page up" }
            ShortcutRow { key: "Home / End"; desc: "Top / bottom of diff" }

            // Editing
            Text {
                Layout.topMargin: theme.sp2
                text: "Selection"
                color: theme.textFaint
                font.family: theme.sans
                font.pixelSize: theme.fontSmall
                font.bold: true
            }

            ShortcutRow { key: "Ctrl+C"; desc: "Copy selected text" }
            ShortcutRow { key: "Ctrl+Shift+C"; desc: "Copy file path" }
            ShortcutRow { key: "Ctrl+A"; desc: "Select all" }

            // Meta
            Text {
                Layout.topMargin: theme.sp2
                text: "General"
                color: theme.textFaint
                font.family: theme.sans
                font.pixelSize: theme.fontSmall
                font.bold: true
            }

            ShortcutRow { key: "?"; desc: "Show this overlay" }
        }
    }

    Keys.onEscapePressed: root.showing = false

    component ShortcutRow: RowLayout {
        property string key: ""
        property string desc: ""
        Layout.fillWidth: true
        spacing: theme.sp3

        Rectangle {
            implicitWidth: Math.max(keyLabel.implicitWidth + theme.sp3, 80)
            implicitHeight: keyLabel.implicitHeight + theme.sp1
            radius: theme.radiusSm
            color: theme.panelStrong
            border.color: theme.borderSoft

            Text {
                id: keyLabel
                anchors.centerIn: parent
                text: parent.parent.key
                color: theme.textStrong
                font.family: theme.mono
                font.pixelSize: theme.fontSmall
                font.bold: true
            }
        }

        Text {
            Layout.fillWidth: true
            text: parent.desc
            color: theme.textBase
            font.family: theme.sans
            font.pixelSize: theme.fontBody
        }
    }
}
