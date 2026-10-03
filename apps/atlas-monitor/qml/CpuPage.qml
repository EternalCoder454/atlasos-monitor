import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

// The processor: load over the last minute, each logical processor's load,
// then what it is (model, cores, caches) and how fast and warm it runs.
AtlasPage {
    id: page

    required property var cpu
    // The refresh interval in ms, for the chart's time caption.
    required property int interval

    // Until the first tick, figures show as a dash, not as 0.
    readonly property bool measured: page.cpu.threads > 0

    title: qsTr("Processor")

    Section {
        title: page.cpu.model

        LiveChart {
            Layout.fillWidth: true
            Layout.preferredHeight: Kirigami.Units.gridUnit * 11
            Layout.margins: Kirigami.Units.largeSpacing
            values: page.cpu.usageHistory
            maximum: 100
            label: qsTr("Load")
            valueText: page.measured ? Format.percent(page.cpu.usage) : ""
            topText: "100%"
            spanText: Format.span(page.interval)
        }
    }

    Section {
        title: qsTr("Each Processor")
        visible: page.cpu.coreUsage.length > 0

        MiniBars {
            Layout.fillWidth: true
            Layout.margins: Kirigami.Units.largeSpacing
            values: page.cpu.coreUsage
            maximum: 100
        }
    }

    Section {
        title: qsTr("Now")

        SectionRow {
            title: qsTr("Speed")
            value: Format.mhz(page.cpu.frequency)
        }
        SectionRow {
            title: qsTr("Temperature")
            value: Format.celsius(page.cpu.temperature)
        }
    }

    Section {
        title: qsTr("Details")

        SectionRow {
            title: qsTr("Base Speed")
            value: Format.mhz(page.cpu.baseFrequency)
        }
        SectionRow {
            title: qsTr("Sockets")
            value: page.measured ? page.cpu.sockets : Format.dash
        }
        SectionRow {
            title: qsTr("Cores")
            value: page.measured ? page.cpu.cores : Format.dash
        }
        SectionRow {
            title: qsTr("Logical Processors")
            value: page.measured ? page.cpu.threads : Format.dash
        }
        SectionRow {
            title: qsTr("L1 Data Cache")
            value: Format.bytes(page.cpu.l1d)
        }
        SectionRow {
            title: qsTr("L1 Instruction Cache")
            value: Format.bytes(page.cpu.l1i)
        }
        SectionRow {
            title: qsTr("L2 Cache")
            value: Format.bytes(page.cpu.l2)
        }
        SectionRow {
            title: qsTr("L3 Cache")
            value: Format.bytes(page.cpu.l3)
        }
    }
}
