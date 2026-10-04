import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

// Memory and swap: use over the last minute, how the memory is shared out
// now, and the figures behind it.
ResourcePage {
    id: page

    required property var memory
    // The refresh interval in ms, for the charts' time caption.
    required property int interval

    readonly property bool measured: page.memory.total > 0
    readonly property bool hasSwap: page.memory.swapTotal > 0
    readonly property color hue: Hues.on(Hues.memory, Kirigami.Theme.backgroundColor)
    readonly property real free: Math.max(0, page.memory.total - page.memory.used - page.memory.cached)

    title: qsTr("Memory")
    headline: page.measured ? Format.percent(page.memory.used / page.memory.total * 100) : Format.dash
    name: page.measured ? qsTr("%1 memory").arg(Format.bytes(page.memory.total)) : qsTr("Memory")

    LiveChart {
        Layout.fillWidth: true
        Layout.preferredHeight: Kirigami.Units.gridUnit * 11
        color: page.hue
        values: page.memory.usageHistory
        maximum: 100
        label: qsTr("Memory usage")
        valueText: page.measured ? Format.share(page.memory.used, page.memory.total) : ""
        topText: "100%"
        spanText: Format.span(page.interval)
    }

    SectionLabel {
        visible: page.measured
        text: qsTr("Memory composition")
    }
    UsageBar {
        Layout.fillWidth: true
        visible: page.measured
        barHeight: Math.round(Kirigami.Units.gridUnit * 1.2)
        colors: [page.hue, Qt.alpha(page.hue, 0.4)]
        total: page.memory.total
        values: [page.memory.used, page.memory.cached]
        labels: [qsTr("In use"), qsTr("Cached"), qsTr("Free")]
        texts: [Format.bytes(page.memory.used), Format.bytes(page.memory.cached), Format.bytes(page.free)]
    }

    SectionLabel {
        visible: page.hasSwap
        text: qsTr("Swap")
    }
    LiveChart {
        Layout.fillWidth: true
        Layout.preferredHeight: Kirigami.Units.gridUnit * 6.5
        visible: page.hasSwap
        color: page.hue
        values: page.memory.swapHistory
        maximum: 100
        label: qsTr("Swap usage")
        valueText: Format.share(page.memory.swapUsed, page.memory.swapTotal)
        topText: "100%"
        spanText: Format.span(page.interval)
    }

    Flow {
        Layout.fillWidth: true
        Layout.topMargin: Kirigami.Units.smallSpacing
        spacing: Kirigami.Units.gridUnit * 2

        GridLayout {
            columns: 3
            columnSpacing: Kirigami.Units.gridUnit * 1.5
            rowSpacing: Kirigami.Units.largeSpacing

            BigStat {
                label: qsTr("In use")
                value: page.measured ? Format.bytes(page.memory.used) : Format.dash
                rule: page.hue
            }
            BigStat {
                label: qsTr("Available")
                value: page.measured ? Format.bytes(page.memory.available) : Format.dash
            }
            BigStat {
                label: qsTr("Cached")
                value: page.measured ? Format.bytes(page.memory.cached) : Format.dash
                rule: Qt.alpha(page.hue, 0.4)
            }
        }

        DetailGrid {
            // No wider than the page: a long address elides.
            width: Math.min(implicitWidth, parent.width)
            entries: [[qsTr("Total"), Format.bytes(page.memory.total)], [qsTr("Free"), page.measured ? Format.bytes(page.free) : Format.dash], [qsTr("Swap total"), page.hasSwap ? Format.bytes(page.memory.swapTotal) : qsTr("None")], [qsTr("Swap in use"), page.hasSwap ? Format.size(page.memory.swapUsed) : ""]]
        }
    }
}
