pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

// One graphics card: its load and video memory over the last minute, then
// clocks, temperatures, fan and power.
ResourcePage {
    id: page

    required property var gpu
    // The refresh interval in ms, for the charts' time caption.
    required property int interval

    readonly property bool hasMemory: page.gpu.memoryTotal > 0
    readonly property color hue: Hues.on(Hues.gpu, Kirigami.Theme.backgroundColor)
    readonly property string memoryName: page.gpu.integrated ? qsTr("Reserved memory") : qsTr("Video memory")

    title: page.gpu.label
    headline: Format.percent(page.gpu.usage)
    name: page.gpu.label

    // A sleeping card is left asleep: reading it would wake it and cost
    // more power than the figures are worth.
    QQC2.Label {
        visible: page.gpu.asleep
        Layout.fillWidth: true
        wrapMode: Text.Wrap
        opacity: 0.7
        text: qsTr("This card is asleep because nothing is using it. Atlas Monitor leaves it asleep rather than wake it to read it, so the figures below are from when it was last awake.")
    }

    LiveChart {
        Layout.fillWidth: true
        Layout.preferredHeight: Kirigami.Units.gridUnit * 11
        color: page.hue
        values: page.gpu.usageHistory
        maximum: 100
        label: qsTr("% Utilization")
        valueText: Format.percent(page.gpu.usage)
        topText: "100%"
        spanText: Format.span(page.interval)
    }
    LiveChart {
        Layout.fillWidth: true
        Layout.preferredHeight: Kirigami.Units.gridUnit * 8.5
        visible: page.hasMemory
        color: page.hue
        values: page.gpu.memoryHistory
        maximum: 100
        label: page.memoryName
        valueText: page.gpu.memoryUsed >= 0 ? Format.share(page.gpu.memoryUsed, page.gpu.memoryTotal) : ""
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
                label: qsTr("Utilization")
                value: Format.percent(page.gpu.usage)
                rule: page.hue
            }
            BigStat {
                visible: page.hasMemory
                label: page.memoryName
                value: page.gpu.memoryUsed >= 0 ? Format.size(page.gpu.memoryUsed) : Format.dash
                rule: page.hue
                dashed: true
            }
            BigStat {
                label: qsTr("Temperature")
                value: Format.celsius(page.gpu.temperature)
            }
            BigStat {
                label: qsTr("Power draw")
                value: Format.watts(page.gpu.power)
            }
            BigStat {
                label: qsTr("GPU clock")
                value: Format.mhz(page.gpu.coreClock)
            }
            BigStat {
                label: qsTr("Fan speed")
                // amdgpu counts the fan's turns; NVIDIA gives its duty cycle.
                value: page.gpu.fanRpm > 0 ? qsTr("%1 RPM").arg(Format.count(page.gpu.fanRpm)) : page.gpu.fanPercent > 0 ? Format.percent(page.gpu.fanPercent) : Format.dash
            }
        }

        DetailGrid {
            // No wider than the page: a long address elides.
            width: Math.min(implicitWidth, parent.width)
            entries: [[qsTr("Memory clock"), Format.mhz(page.gpu.memoryClock)], [qsTr("Hotspot"), isNaN(page.gpu.hotspot) ? "" : Format.celsius(page.gpu.hotspot)], [qsTr("Memory temperature"), isNaN(page.gpu.memoryTemperature) ? "" : Format.celsius(page.gpu.memoryTemperature)], [qsTr("Power limit"), page.gpu.powerLimit > 0 ? Format.watts(page.gpu.powerLimit) : ""], [qsTr("%1 total").arg(page.memoryName), page.hasMemory ? Format.bytes(page.gpu.memoryTotal) : ""], [qsTr("Shared memory"), page.gpu.gttUsed < 0 ? "" : page.gpu.gttTotal > 0 ? Format.share(page.gpu.gttUsed, page.gpu.gttTotal) : Format.size(page.gpu.gttUsed)], [qsTr("Driver"), page.gpu.driver], [qsTr("PCI slot"), page.gpu.slot]]
        }
    }
}
