import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

// One graphics card: its load and video memory over the last minute, then
// clocks, temperatures, fan and power.
AtlasPage {
    id: page

    required property var gpu
    // The refresh interval in ms, for the charts' time caption.
    required property int interval

    readonly property bool hasMemory: page.gpu.memoryTotal > 0

    title: page.gpu.label

    // A sleeping card is left asleep: reading it would wake it and cost
    // more power than the figures are worth.
    QQC2.Label {
        visible: page.gpu.asleep
        Layout.fillWidth: true
        Layout.bottomMargin: Kirigami.Units.largeSpacing
        wrapMode: Text.Wrap
        opacity: 0.7
        text: qsTr("This card is asleep because nothing is using it. Atlas Monitor leaves it asleep rather than wake it to read it.")
    }

    Section {
        title: qsTr("Load")

        LiveChart {
            Layout.fillWidth: true
            Layout.preferredHeight: Kirigami.Units.gridUnit * 10
            Layout.margins: Kirigami.Units.largeSpacing
            values: page.gpu.usageHistory
            maximum: 100
            label: qsTr("In Use")
            valueText: Format.percent(page.gpu.usage)
            topText: "100%"
            spanText: Format.span(page.interval)
        }
    }

    Section {
        title: page.gpu.integrated ? qsTr("Reserved Memory") : qsTr("Video Memory")
        visible: page.hasMemory

        LiveChart {
            Layout.fillWidth: true
            Layout.preferredHeight: Kirigami.Units.gridUnit * 6
            Layout.margins: Kirigami.Units.largeSpacing
            values: page.gpu.memoryHistory
            maximum: 100
            label: qsTr("In Use")
            valueText: page.gpu.memoryUsed >= 0 ? Format.share(page.gpu.memoryUsed, page.gpu.memoryTotal) : ""
            topText: "100%"
            spanText: Format.span(page.interval)
        }
    }

    Section {
        title: qsTr("Details")

        SectionRow {
            title: qsTr("GPU Clock")
            value: Format.mhz(page.gpu.coreClock)
        }
        SectionRow {
            title: qsTr("Memory Clock")
            value: Format.mhz(page.gpu.memoryClock)
        }
        SectionRow {
            title: qsTr("Temperature")
            value: Format.celsius(page.gpu.temperature)
        }
        SectionRow {
            visible: !isNaN(page.gpu.hotspot)
            title: qsTr("Hotspot")
            value: Format.celsius(page.gpu.hotspot)
        }
        SectionRow {
            visible: !isNaN(page.gpu.memoryTemperature)
            title: qsTr("Memory Temperature")
            value: Format.celsius(page.gpu.memoryTemperature)
        }
        SectionRow {
            title: qsTr("Fan Speed")
            // amdgpu counts the fan's turns; NVIDIA gives its duty cycle.
            value: page.gpu.fanRpm > 0 ? qsTr("%1 RPM").arg(Format.count(page.gpu.fanRpm)) : page.gpu.fanPercent > 0 ? Format.percent(page.gpu.fanPercent) : Format.dash
        }
        SectionRow {
            title: qsTr("Power Draw")
            value: page.gpu.powerLimit > 0 && page.gpu.power > 0 ? qsTr("%1 of %2").arg(Format.watts(page.gpu.power)).arg(Format.watts(page.gpu.powerLimit)) : Format.watts(page.gpu.power)
        }
        SectionRow {
            title: page.gpu.integrated ? qsTr("Reserved Memory Used") : qsTr("Video Memory Used")
            value: page.hasMemory && page.gpu.memoryUsed >= 0 ? Format.share(page.gpu.memoryUsed, page.gpu.memoryTotal) : Format.dash
        }
        SectionRow {
            visible: page.gpu.gttUsed >= 0
            title: qsTr("Shared Memory Used")
            value: page.gpu.gttTotal > 0 ? Format.share(page.gpu.gttUsed, page.gpu.gttTotal) : Format.size(page.gpu.gttUsed)
        }
        SectionRow {
            visible: page.gpu.driver.length > 0
            title: qsTr("Driver")
            value: page.gpu.driver
        }
        SectionRow {
            visible: page.gpu.slot.length > 0
            title: qsTr("PCI Slot")
            value: page.gpu.slot
        }
    }
}
