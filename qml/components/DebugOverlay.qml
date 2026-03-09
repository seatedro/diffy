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
    property string worstOp: "-"
    property real worstOpMs: 0

    function updateWorst(name, ms) {
        if (ms > root.worstOpMs) {
            root.worstOpMs = ms
            root.worstOp = name
        }
    }

    Timer {
        running: root.showing
        interval: 500
        repeat: true
        onTriggered: {
            root.fps = root.frameCount * (1000.0 / interval)
            root.frameCount = 0
            root.peakPaintMs = 0
            root.peakRasterMs = 0
            root.worstOp = "-"
            root.worstOpMs = 0
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
                var ut = root.surface.lastTextureUploadTimeMs
                if (pt > root.peakPaintMs) root.peakPaintMs = pt
                if (rt > root.peakRasterMs) root.peakRasterMs = rt
                root.updateWorst("paint", pt)
                root.updateWorst("raster", rt)
                root.updateWorst("upload", ut)
            }
        }
        function onPerfStatsChanged() {
            if (root.surface) {
                root.updateWorst("rebuild", root.surface.lastRowsRebuildTimeMs)
                root.updateWorst("layout", root.surface.lastDisplayRowsRebuildTimeMs)
                root.updateWorst("metrics", root.surface.lastMetricsRecalcTimeMs)
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
            property real uploadMs: root.surface ? root.surface.lastTextureUploadTimeMs : 0
            text: "upload  " + uploadMs.toFixed(2) + " ms"
            font.family: theme.mono
            font.pixelSize: 10
            color: uploadMs > 4 ? "#ff6060" : "#cccccc"
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
            text: "worst op    " + root.worstOp + " " + root.worstOpMs.toFixed(2) + " ms"
            font.family: theme.mono
            font.pixelSize: 10
            color: root.worstOpMs > 16 ? "#ff6060" : root.worstOpMs > 8 ? "#ffcc44" : "#999999"
        }

        Rectangle { width: col.width; height: 1; color: "#20ffffff" }

        Text {
            property int hits: root.surface ? root.surface.tileCacheHits : 0
            property int misses: root.surface ? root.surface.tileCacheMisses : 0
            property real rate: (hits + misses) > 0 ? (100.0 * hits / (hits + misses)) : 0
            text: "cache   " + rate.toFixed(0) + "% (" + hits + "/" + (hits + misses) + ")"
            font.family: theme.mono
            font.pixelSize: 10
            color: rate < 80 ? "#ffcc44" : "#cccccc"
        }

        Text {
            property int resident: root.surface ? root.surface.residentTileCount : 0
            text: "tiles   " + resident
            font.family: theme.mono
            font.pixelSize: 10
            color: "#cccccc"
        }

        Text {
            property int pending: root.surface ? root.surface.pendingTileJobCount : 0
            text: "pending " + pending
            font.family: theme.mono
            font.pixelSize: 10
            color: pending > 10 ? "#ffcc44" : "#cccccc"
        }

        Text {
            property int uploads: root.surface ? root.surface.textureUploadCount : 0
            text: "uploads " + uploads
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

        Text {
            property int paints: root.surface ? root.surface.paintCount : 0
            text: "frames  " + paints
            font.family: theme.mono
            font.pixelSize: 10
            color: "#cccccc"
        }

        Text {
            property real rebuildMs: root.surface ? root.surface.lastRowsRebuildTimeMs : 0
            text: "rebuild " + rebuildMs.toFixed(2) + " ms"
            font.family: theme.mono
            font.pixelSize: 10
            color: rebuildMs > 50 ? "#ff6060" : rebuildMs > 10 ? "#ffcc44" : "#cccccc"
        }

        Text {
            property real metricsMs: root.surface ? root.surface.lastMetricsRecalcTimeMs : 0
            text: "metrics " + metricsMs.toFixed(2) + " ms"
            font.family: theme.mono
            font.pixelSize: 10
            color: metricsMs > 50 ? "#ff6060" : metricsMs > 10 ? "#ffcc44" : "#cccccc"
        }
    }
}
