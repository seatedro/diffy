.pragma library

function linearizeChannel(channel) {
    if (channel <= 0.03928)
        return channel / 12.92
    return Math.pow((channel + 0.055) / 1.055, 2.4)
}

function relativeLuminance(color) {
    return 0.2126 * linearizeChannel(color.r)
        + 0.7152 * linearizeChannel(color.g)
        + 0.0722 * linearizeChannel(color.b)
}

function contrastRatio(foreground, background) {
    var fg = relativeLuminance(foreground)
    var bg = relativeLuminance(background)
    var lighter = Math.max(fg, bg)
    var darker = Math.min(fg, bg)
    return (lighter + 0.05) / (darker + 0.05)
}

function bestContrastColor(background, candidates) {
    if (!candidates || candidates.length === 0)
        return background

    var best = candidates[0]
    var bestRatio = contrastRatio(best, background)
    for (var i = 1; i < candidates.length; ++i) {
        var candidate = candidates[i]
        var ratio = contrastRatio(candidate, background)
        if (ratio > bestRatio) {
            best = candidate
            bestRatio = ratio
        }
    }
    return best
}

function withAlpha(color, alpha) {
    return Qt.rgba(color.r, color.g, color.b, alpha)
}
