import QtQuick

Rectangle {
    id: root

    property string text: ""
    property string variant: "neutral" // neutral, success, danger, warning, accent

    function bgColor() {
        if (variant === "success") return theme.successBg
        if (variant === "danger") return theme.dangerBg
        if (variant === "warning") return theme.warningBg
        if (variant === "accent") return theme.accentSoft
        return theme.panelStrong
    }

    function borderCol() {
        if (variant === "success") return theme.successBorder
        if (variant === "danger") return theme.dangerBorder
        if (variant === "warning") return theme.warningBorder
        if (variant === "accent") return theme.selectionBorder
        return theme.borderSoft
    }

    function textCol() {
        if (variant === "success") return theme.successText
        if (variant === "danger") return theme.dangerText
        if (variant === "warning") return theme.warningText
        if (variant === "accent") return theme.accentStrong
        return theme.textMuted
    }

    implicitWidth: label.implicitWidth + theme.sp3
    implicitHeight: label.implicitHeight + theme.sp1
    radius: theme.radiusSm
    color: bgColor()
    border.color: borderCol()
    border.width: 1

    Text {
        id: label
        anchors.centerIn: parent
        text: root.text
        color: root.textCol()
        font.family: theme.sans
        font.pixelSize: theme.fontCaption
        font.bold: true
    }
}
