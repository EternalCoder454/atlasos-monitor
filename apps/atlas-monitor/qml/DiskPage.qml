pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

// One drive: how full it is, what it reads and writes, and what it says
// about its own health.
ResourcePage {
    id: page

    required property var disk
    // The refresh interval in ms, for the charts' time caption.
    required property int interval

    readonly property color hue: Hues.on(Hues.disk, Kirigami.Theme.backgroundColor)

    title: page.disk.label
    headline: page.disk.label
    headlineScale: 2.1
    name: [Format.bytes(page.disk.size), page.disk.name].filter(t => t.length > 0 && t !== Format.dash).join(" · ")

    // zram is RAM, not a drive: say what it is instead of a capacity.
    QQC2.Label {
        visible: page.disk.swap
        Layout.fillWidth: true
        wrapMode: Text.Wrap
        opacity: 0.7
        text: qsTr("Swap (zram) is a compressed pool carved out of your RAM that acts as overflow memory: when RAM fills up, the kernel compresses rarely used pages and parks them here instead of writing them to your SSD. That keeps the system responsive under pressure and spares the drive.")
    }

    SectionLabel {
        visible: !page.disk.swap
        text: qsTr("Capacity")
    }
    UsageBar {
        Layout.fillWidth: true
        visible: !page.disk.swap && page.disk.mounted
        barHeight: Math.round(Kirigami.Units.gridUnit * 1.2)
        colors: [page.hue]
        total: page.disk.used + page.disk.free
        values: [page.disk.used]
        labels: [qsTr("Used"), qsTr("Free")]
        texts: [Format.size(page.disk.used), Format.size(page.disk.free)]
    }
    // Not mounted: there is no usage to show, and 0 B used would read as an
    // empty drive.
    QQC2.Label {
        visible: !page.disk.swap && !page.disk.mounted
        Layout.fillWidth: true
        wrapMode: Text.Wrap
        opacity: 0.7
        text: qsTr("Not mounted, so there is no usage to show. Mount the drive in your file manager and its capacity will appear here.")
    }

    LiveChart {
        Layout.fillWidth: true
        Layout.preferredHeight: Kirigami.Units.gridUnit * 8.5
        color: page.hue
        values: page.disk.readHistory
        // A quiet drive still shows a scale of a megabyte a second, so a
        // few kilobytes don't fill the chart.
        minimumScale: 1048576
        label: qsTr("Read speed")
        valueText: Format.rate(page.disk.readRate)
        topText: Format.rate(scaleTop)
        spanText: Format.span(page.interval)
    }
    LiveChart {
        Layout.fillWidth: true
        Layout.preferredHeight: Kirigami.Units.gridUnit * 8.5
        color: page.hue
        values: page.disk.writeHistory
        minimumScale: 1048576
        label: qsTr("Write speed")
        valueText: Format.rate(page.disk.writeRate)
        topText: Format.rate(scaleTop)
        spanText: Format.span(page.interval)
    }

    Flow {
        Layout.fillWidth: true
        Layout.topMargin: Kirigami.Units.smallSpacing
        spacing: Kirigami.Units.gridUnit * 2

        GridLayout {
            columns: 2
            columnSpacing: Kirigami.Units.gridUnit * 1.5
            rowSpacing: Kirigami.Units.largeSpacing

            BigStat {
                label: qsTr("Read speed")
                value: Format.rate(page.disk.readRate)
                rule: page.hue
            }
            BigStat {
                label: qsTr("Write speed")
                value: Format.rate(page.disk.writeRate)
                rule: page.hue
                dashed: true
            }
            BigStat {
                visible: !page.disk.swap && page.disk.mounted
                label: qsTr("Used")
                value: page.disk.mounted ? Format.size(page.disk.used) : Format.dash
            }
            BigStat {
                visible: !page.disk.swap && page.disk.mounted
                label: qsTr("Free")
                value: page.disk.mounted ? Format.size(page.disk.free) : Format.dash
            }
        }

        DetailGrid {
            // No wider than the page: a long address elides.
            width: Math.min(implicitWidth, parent.width)
            entries: [[qsTr("Total size"), Format.bytes(page.disk.size)], [qsTr("Read since startup"), Format.size(page.disk.readTotal)], [qsTr("Written since startup"), Format.size(page.disk.writeTotal)], [qsTr("Device"), "/dev/" + page.disk.name]]
        }
    }

    SectionLabel {
        visible: page.disk.smart
        Layout.topMargin: Kirigami.Units.largeSpacing
        text: qsTr("Health")
    }
    QQC2.Label {
        visible: page.disk.smart && page.disk.warning.length > 0
        Layout.fillWidth: true
        wrapMode: Text.Wrap
        color: Kirigami.Theme.negativeTextColor
        text: page.disk.warning
    }
    DetailGrid {
        Layout.fillWidth: true
        visible: page.disk.smart
        pairs: page.width >= Kirigami.Units.gridUnit * 34 ? 2 : 1
        // As the Go version: a figure the drive reports as 0 says nothing
        // yet, and the error counts show only once there are some.
        entries: [[qsTr("Life remaining"), isNaN(page.disk.life) ? "" : page.disk.life <= 10 ? qsTr("%1 · replace it").arg(Format.percent(page.disk.life)) : page.disk.life <= 30 ? qsTr("%1 · wearing out").arg(Format.percent(page.disk.life)) : Format.percent(page.disk.life)], [qsTr("Temperature"), isNaN(page.disk.temperature) ? "" : Format.celsius(page.disk.temperature)], [qsTr("Powered on"), page.disk.powerOnHours > 0 ? Format.hours(page.disk.powerOnHours) : ""], [qsTr("Written in total"), page.disk.written > 0 ? Format.size(page.disk.written) : ""], [qsTr("Power cycles"), page.disk.powerCycles > 0 ? Format.count(page.disk.powerCycles) : ""], [qsTr("Unsafe shutdowns"), page.disk.unsafeShutdowns > 0 ? Format.count(page.disk.unsafeShutdowns) : ""], [qsTr("Media errors"), page.disk.mediaErrors > 0 ? Format.count(page.disk.mediaErrors) : ""]]
    }
}
