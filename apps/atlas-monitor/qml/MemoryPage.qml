import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

// Memory and swap: use over the last minute, how the memory is shared out
// now, and the figures behind it.
AtlasPage {
    id: page

    required property var memory
    // The refresh interval in ms, for the charts' time caption.
    required property int interval

    readonly property bool measured: page.memory.total > 0
    readonly property bool hasSwap: page.memory.swapTotal > 0

    title: qsTr("Memory")

    Section {
        LiveChart {
            Layout.fillWidth: true
            Layout.preferredHeight: Kirigami.Units.gridUnit * 10
            Layout.margins: Kirigami.Units.largeSpacing
            values: page.memory.usageHistory
            maximum: 100
            label: qsTr("In Use")
            valueText: page.measured ? Format.share(page.memory.used, page.memory.total) : ""
            topText: "100%"
            spanText: Format.span(page.interval)
        }

        UsageBar {
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.largeSpacing
            Layout.rightMargin: Kirigami.Units.largeSpacing
            Layout.bottomMargin: Kirigami.Units.largeSpacing
            visible: page.measured
            total: page.memory.total
            values: [page.memory.used, page.memory.cached]
            labels: [qsTr("In Use"), qsTr("Cached"), qsTr("Free")]
            texts: [Format.bytes(page.memory.used), Format.bytes(page.memory.cached), Format.bytes(Math.max(0, page.memory.total - page.memory.used - page.memory.cached))]
        }
    }

    Section {
        title: qsTr("Swap")
        visible: page.hasSwap

        LiveChart {
            Layout.fillWidth: true
            Layout.preferredHeight: Kirigami.Units.gridUnit * 6
            Layout.margins: Kirigami.Units.largeSpacing
            values: page.memory.swapHistory
            maximum: 100
            label: qsTr("In Use")
            valueText: Format.share(page.memory.swapUsed, page.memory.swapTotal)
            topText: "100%"
            spanText: Format.span(page.interval)
        }
    }

    Section {
        title: qsTr("Details")

        SectionRow {
            title: qsTr("Total")
            value: Format.bytes(page.memory.total)
        }
        SectionRow {
            title: qsTr("In Use")
            value: page.measured ? Format.bytes(page.memory.used) : Format.dash
        }
        SectionRow {
            title: qsTr("Cached")
            value: page.measured ? Format.bytes(page.memory.cached) : Format.dash
        }
        SectionRow {
            title: qsTr("Available")
            value: page.measured ? Format.bytes(page.memory.available) : Format.dash
        }
        SectionRow {
            title: qsTr("Swap Total")
            value: page.hasSwap ? Format.bytes(page.memory.swapTotal) : qsTr("None")
        }
        SectionRow {
            visible: page.hasSwap
            title: qsTr("Swap In Use")
            value: Format.size(page.memory.swapUsed)
        }
    }
}
