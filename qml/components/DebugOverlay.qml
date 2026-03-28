import QtQuick

Rectangle {
    id: root

    property var surface: null
    property bool showing: false

    visible: showing
    anchors.top: parent.top
    anchors.right: parent.right
    anchors.topMargin: 8
    anchors.rightMargin: 8
    z: 999

    width: col.width + 20
    height: col.height + 16
    radius: 6
    color: "#e0101010"
    border.width: 1
    border.color: "#30ffffff"

    property int frameCount: 0
    property real fps: 0
    property real peakPaintMs: 0
    property real peakRasterMs: 0
    property real peakLayoutMs: 0

    Timer {
        running: root.showing
        interval: 500
        repeat: true
        onTriggered: {
            root.fps = root.frameCount * (1000.0 / interval)
            root.frameCount = 0
            root.peakPaintMs = 0
            root.peakRasterMs = 0
            root.peakLayoutMs = 0
        }
    }

    Connections {
        target: root.surface
        enabled: root.showing && root.surface !== null
        function onPaintCountChanged() {
            root.frameCount++
            if (root.surface) {
                var pt = root.surface.lastPaintTimeMs
                var rt = root.surface.lastRasterTimeMs
                if (pt > root.peakPaintMs) root.peakPaintMs = pt
                if (rt > root.peakRasterMs) root.peakRasterMs = rt
            }
        }
        function onPerfStatsChanged() {
            if (root.surface) {
                var lt = root.surface.lastLayoutTimeMs
                if (lt > root.peakLayoutMs) root.peakLayoutMs = lt
            }
        }
    }

    Column {
        id: col
        x: 10
        y: 8
        spacing: 2

        Text {
            text: "DEBUG"
            font.family: theme.mono
            font.pixelSize: 9
            font.bold: true
            color: "#80ff80"
        }

        Rectangle { width: col.width; height: 1; color: "#30ffffff" }

        Text {
            text: "fps     " + root.fps.toFixed(0)
            font.family: theme.mono
            font.pixelSize: 10
            color: root.fps < 30 ? "#ff6060" : root.fps < 55 ? "#ffcc44" : "#cccccc"
        }

        Text {
            property real paintMs: root.surface ? root.surface.lastPaintTimeMs : 0
            text: "paint   " + paintMs.toFixed(2) + " ms"
            font.family: theme.mono
            font.pixelSize: 10
            color: paintMs > 8 ? "#ff6060" : paintMs > 4 ? "#ffcc44" : "#cccccc"
        }

        Text {
            property real rasterMs: root.surface ? root.surface.lastRasterTimeMs : 0
            text: "raster  " + rasterMs.toFixed(2) + " ms"
            font.family: theme.mono
            font.pixelSize: 10
            color: rasterMs > 4 ? "#ff6060" : rasterMs > 2 ? "#ffcc44" : "#cccccc"
        }

        Text {
            property real layoutMs: root.surface ? root.surface.lastLayoutTimeMs : 0
            text: "layout  " + layoutMs.toFixed(2) + " ms"
            font.family: theme.mono
            font.pixelSize: 10
            color: layoutMs > 8 ? "#ff6060" : layoutMs > 4 ? "#ffcc44" : "#cccccc"
        }

        Rectangle { width: col.width; height: 1; color: "#20ffffff" }

        Text {
            text: "peak paint  " + root.peakPaintMs.toFixed(2) + " ms"
            font.family: theme.mono
            font.pixelSize: 10
            color: root.peakPaintMs > 16 ? "#ff6060" : "#999999"
        }

        Text {
            text: "peak raster " + root.peakRasterMs.toFixed(2) + " ms"
            font.family: theme.mono
            font.pixelSize: 10
            color: root.peakRasterMs > 8 ? "#ff6060" : "#999999"
        }

        Text {
            text: "peak layout " + root.peakLayoutMs.toFixed(2) + " ms"
            font.family: theme.mono
            font.pixelSize: 10
            color: root.peakLayoutMs > 16 ? "#ff6060" : root.peakLayoutMs > 8 ? "#ffcc44" : "#999999"
        }

        Rectangle { width: col.width; height: 1; color: "#20ffffff" }

        Text {
            property int strips: root.surface ? root.surface.stripCount : 0
            text: "strips  " + strips
            font.family: theme.mono
            font.pixelSize: 10
            color: "#cccccc"
        }

        Text {
            property int reuse: root.surface ? root.surface.stripReuseCount : 0
            text: "reuse   " + reuse
            font.family: theme.mono
            font.pixelSize: 10
            color: "#cccccc"
        }

        Text {
            property int rastered: root.surface ? root.surface.stripRerasterCount : 0
            text: "rasterd " + rastered
            font.family: theme.mono
            font.pixelSize: 10
            color: "#cccccc"
        }

        Rectangle { width: col.width; height: 1; color: "#20ffffff" }

        Text {
            property int rows: root.surface ? root.surface.displayRowCount : 0
            text: "rows    " + rows
            font.family: theme.mono
            font.pixelSize: 10
            color: "#cccccc"
        }
    }
}
