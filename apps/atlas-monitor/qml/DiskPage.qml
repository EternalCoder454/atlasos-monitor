import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

// One drive: how full it is, what it reads and writes, and what it says
// about its own health.
AtlasPage {
    id: page

    required property var disk
    // The refresh interval in ms, for the chart's time caption.
    required property int interval

    title: page.disk.label

    // zram is RAM, not a drive: say what it is instead of a capacity.
    QQC2.Label {
        visible: page.disk.swap
        Layout.fillWidth: true
        Layout.bottomMargin: Kirigami.Units.largeSpacing
        wrapMode: Text.Wrap
        opacity: 0.7
        text: qsTr("Swap (zram) is a compressed pool carved out of your RAM that acts as overflow memory: when RAM fills up, the kernel compresses rarely used pages and parks them here instead of writing them to your SSD. That keeps the system responsive under pressure and spares the drive.")
    }

    Section {
        title: qsTr("Capacity")
        visible: !page.disk.swap
        // Not mounted: there is no usage to show, and 0 B used would read as
        // an empty drive.
        footer: page.disk.mounted ? "" : qsTr("Not mounted, so there is no usage to show. Mount the drive in your file manager and its capacity will appear here.")

        UsageBar {
            Layout.fillWidth: true
            Layout.margins: Kirigami.Units.largeSpacing
            visible: page.disk.mounted
            total: page.disk.used + page.disk.free
            values: [page.disk.used]
            labels: [qsTr("Used"), qsTr("Free")]
            texts: [Format.size(page.disk.used), Format.size(page.disk.free)]
        }
    }

    Section {
        title: qsTr("Activity")

        LiveChart {
            Layout.fillWidth: true
            Layout.preferredHeight: Kirigami.Units.gridUnit * 10
            Layout.margins: Kirigami.Units.largeSpacing
            values: page.disk.readHistory
            values2: page.disk.writeHistory
            // A quiet drive still shows a scale of a megabyte a second, so
            // a few kilobytes don't fill the chart.
            minimumScale: 1048576
            label: qsTr("Read")
            valueText: Format.rate(page.disk.readRate) + "   " + qsTr("Write") + " " + Format.rate(page.disk.writeRate)
            topText: Format.rate(scaleTop)
            spanText: Format.span(page.interval)
        }
    }

    Section {
        title: qsTr("Health")
        visible: page.disk.smart
        footer: page.disk.warning

        SectionRow {
            visible: !isNaN(page.disk.life)
            title: qsTr("Life Remaining")
            value: page.disk.life <= 10 ? qsTr("%1 · replace it").arg(Format.percent(page.disk.life)) : page.disk.life <= 30 ? qsTr("%1 · wearing out").arg(Format.percent(page.disk.life)) : Format.percent(page.disk.life)
        }
        SectionRow {
            visible: !isNaN(page.disk.temperature)
            title: qsTr("Temperature")
            value: Format.celsius(page.disk.temperature)
        }
        SectionRow {
            // As the Go version: a drive that reports 0 says nothing yet.
            visible: page.disk.powerOnHours > 0
            title: qsTr("Powered On")
            value: Format.hours(page.disk.powerOnHours)
        }
        SectionRow {
            visible: page.disk.written > 0
            title: qsTr("Written in Total")
            value: Format.size(page.disk.written)
        }
        SectionRow {
            visible: page.disk.powerCycles > 0
            title: qsTr("Power Cycles")
            value: Format.count(page.disk.powerCycles)
        }
        SectionRow {
            // Every drive reports 0 of these until something goes wrong.
            visible: page.disk.unsafeShutdowns > 0
            title: qsTr("Unsafe Shutdowns")
            value: Format.count(page.disk.unsafeShutdowns)
        }
        SectionRow {
            visible: page.disk.mediaErrors > 0
            title: qsTr("Media Errors")
            value: Format.count(page.disk.mediaErrors)
        }
    }

    Section {
        title: qsTr("Details")

        SectionRow {
            title: qsTr("Total Size")
            value: Format.bytes(page.disk.size)
        }
        SectionRow {
            visible: !page.disk.swap
            title: qsTr("Used")
            value: page.disk.mounted ? Format.size(page.disk.used) : Format.dash
        }
        SectionRow {
            visible: !page.disk.swap
            title: qsTr("Free")
            value: page.disk.mounted ? Format.size(page.disk.free) : Format.dash
        }
        SectionRow {
            title: qsTr("Read Since Startup")
            value: Format.size(page.disk.readTotal)
        }
        SectionRow {
            title: qsTr("Written Since Startup")
            value: Format.size(page.disk.writeTotal)
        }
        SectionRow {
            title: qsTr("Device")
            value: "/dev/" + page.disk.name
        }
    }
}
