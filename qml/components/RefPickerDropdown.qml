import QtQuick

Item {
    id: root

    property bool showing: false
    property string target: "left"
    property Item anchorItem: null
    property bool externalInputMode: false

    signal refSelected(string target, string value)

    function open(anchorTarget, anchorElement, initialQuery, useExternalInput, passthroughElement) {
        target = anchorTarget
        anchorItem = anchorElement
        externalInputMode = useExternalInput === true
        combo.searchFieldVisible = !externalInputMode
        combo.passthroughItem = externalInputMode ? (passthroughElement || anchorElement) : null
        combo.show(anchorElement, initialQuery === undefined ? "" : initialQuery, !externalInputMode)
        refreshQuery(combo.searchText)
        showing = true
    }

    function close() {
        combo.hide()
        combo.searchFieldVisible = true
        combo.passthroughItem = null
        externalInputMode = false
        showing = false
    }

    function refreshQuery(query) {
        var mode = root.detectMode(query)
        if (mode === "commit" && query.length >= 4) {
            commitSearchTimer.restart()
            return
        }

        commitSearchTimer.stop()
        combo.model = root.buildModel(query)
        combo.selectedIndex = root.firstSelectable(combo.model)
        combo.footerText = root.countBranches(combo.model) + " branches"
    }

    function syncQuery(activeTarget, query) {
        if (!showing || target !== activeTarget)
            return

        if (combo.searchText !== query)
            combo.searchText = query
        refreshQuery(query)
    }

    function detectMode(text) {
        if (text.length === 0) return "branches"
        if (/^[0-9a-fA-F]{4,40}$/.test(text)) return "commit"
        if (/^v\d/.test(text)) return "tag"
        return "filter"
    }

    function buildModel(query) {
        var mode = detectMode(query)
        var items = []
        var branches = diffController.branches

        if (mode === "commit" && query.length >= 4) {
            var commits = diffController.searchCommits(query)
            if (commits.length > 0) {
                items.push({isHeader: true, label: "Commits"})
                for (var c = 0; c < commits.length; ++c)
                    items.push({label: commits[c].oid.substring(0, 10), detail: commits[c].summary, value: commits[c].oid})
            } else {
                items.push({isHeader: true, label: "No matching commits"})
            }
            return items
        }

        if (mode === "tag") {
            var tags = diffController.tags
            if (tags.length > 0) {
                var tagFiltered = diffController.fuzzyFilter(query, tags, "name")
                if (tagFiltered.length > 0) {
                    items.push({isHeader: true, label: "Tags"})
                    for (var t = 0; t < tagFiltered.length; ++t)
                        items.push({label: tagFiltered[t].name, detail: "tag", value: tagFiltered[t].name})
                }
            }
        }

        var filtered
        if (query.length > 0 && mode === "filter")
            filtered = diffController.fuzzyFilter(query, branches, "name")
        else if (mode !== "tag")
            filtered = branches
        else
            filtered = []

        var local = [], remote = []
        for (var i = 0; i < filtered.length; ++i) {
            var b = filtered[i]
            var entry = {label: b.name, badge: b.isHead ? "HEAD" : "", detail: "", value: b.name}
            if (b.isRemote) remote.push(entry)
            else local.push(entry)
        }

        if (query.length === 0) {
            var recents = diffController.recentBranchesForRepo()
            if (recents.length > 0) {
                items.push({isHeader: true, label: "Recent"})
                for (var r = 0; r < recents.length; ++r)
                    items.push({label: recents[r].name, detail: "", value: recents[r].name})
            }
        }

        if (local.length > 0) {
            items.push({isHeader: true, label: "Local Branches"})
            for (var l = 0; l < local.length; ++l) items.push(local[l])
        }
        if (remote.length > 0) {
            items.push({isHeader: true, label: "Remote Branches"})
            for (var rm = 0; rm < remote.length; ++rm) items.push(remote[rm])
        }
        return items
    }

    function firstSelectable(items) {
        for (var i = 0; i < items.length; ++i)
            if (!items[i].isHeader) return i
        return -1
    }

    function countBranches(items) {
        var n = 0
        for (var i = 0; i < items.length; ++i)
            if (!items[i].isHeader) ++n
        return n
    }

    visible: showing
    anchors.fill: parent
    z: 300

    Timer {
        id: commitSearchTimer
        interval: 200
        repeat: false
        onTriggered: {
            combo.model = root.buildModel(combo.searchText)
            combo.selectedIndex = root.firstSelectable(combo.model)
            combo.footerText = root.countBranches(combo.model) + " branches"
        }
    }

    FilterComboBox {
        id: combo
        anchors.fill: parent
        placeholder: "Filter branches\u2026"

        onSearchTextChanged: {
            root.refreshQuery(searchText)
        }

        onItemSelected: function(item) {
            root.refSelected(root.target, item.value)
            root.close()
        }

        onOpenChanged: {
            if (!open) root.showing = false
        }
    }
}
