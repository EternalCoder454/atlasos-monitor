pragma Singleton

import QtQuick

// Each resource's colour, as the Go version and Task Manager have them: one
// hue per kind of thing, on its page's charts and bars and on its Overview
// tile, so a glance tells the processor from the network.
QtObject {
    readonly property color cpu: "#39b8e3"
    readonly property color memory: "#5c9efa"
    readonly property color disk: "#84c718"
    readonly property color network: "#f5628e"
    readonly property color gpu: "#de68f2"
    readonly property color battery: "#33d17a"
    readonly property color power: "#f6d32d"

    // The hue to draw with on `background`: these are bright, too faint
    // for a line on a light theme, so there they are darkened as the Go
    // version's (to 72%).
    function on(hue, background) {
        return background.hslLightness > 0.5 ? Qt.rgba(hue.r * 0.72, hue.g * 0.72, hue.b * 0.72, 1) : hue;
    }
}
