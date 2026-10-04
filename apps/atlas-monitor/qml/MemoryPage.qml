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
    subtitle: page.measured ? qsTr("%1 installed").arg(Format.bytes(page.memory.total)) : ""
    figure: page.measured ? Format.percent(page.memory.used / page.memory.total * 100) : Format.dash
    figureColor: page.hue

    MonitorCard {
        LiveChart {
            Layout.fillWidth: true
            Layout.preferredHeight: Kirigami.Units.gridUnit * 9
            color: page.hue
            values: page.memory.usageHistory
            maximum: 100
            label: qsTr("In use")
            valueText: page.measured ? Format.share(page.memory.used, page.memory.total) : ""
            topText: "100%"
            spanText: Format.span(page.interval)
        }
        UsageBar {
            Layout.fillWidth: true
            visible: page.measured
            barHeight: Math.round(Kirigami.Units.gridUnit * 0.9)
            colors: [page.hue, Qt.alpha(page.hue, 0.4)]
            total: page.memory.total
            values: [page.memory.used, page.memory.cached]
            labels: [qsTr("In use"), qsTr("Cached"), qsTr("Free")]
            texts: [Format.bytes(page.memory.used), Format.bytes(page.memory.cached), Format.bytes(page.free)]
        }
    }

    MonitorCard {
        visible: page.hasSwap
        title: qsTr("Swap")

        LiveChart {
            Layout.fillWidth: true
            Layout.preferredHeight: Kirigami.Units.gridUnit * 5.5
            color: page.hue
            values: page.memory.swapHistory
            maximum: 100
            label: qsTr("In use")
            valueText: Format.share(page.memory.swapUsed, page.memory.swapTotal)
            topText: "100%"
            spanText: Format.span(page.interval)
        }
    }

    FigureCard {
        BigStat {
            label: qsTr("In use")
            value: page.measured ? Format.bytes(page.memory.used) : Format.dash
        }
        BigStat {
            label: qsTr("Available")
            value: page.measured ? Format.bytes(page.memory.available) : Format.dash
        }
        BigStat {
            label: qsTr("Cached")
            value: page.measured ? Format.bytes(page.memory.cached) : Format.dash
        }
        details: [[qsTr("Total"), Format.bytes(page.memory.total)], [qsTr("Free"), page.measured ? Format.bytes(page.free) : Format.dash], [qsTr("Swap total"), page.hasSwap ? Format.bytes(page.memory.swapTotal) : qsTr("None")], [qsTr("Swap in use"), page.hasSwap ? Format.size(page.memory.swapUsed) : ""]]
    }
}
