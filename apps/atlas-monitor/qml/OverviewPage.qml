pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

// The whole computer at a glance, as Task Manager's performance list: one
// line on what, if anything, is wrong, then a tile per part with its last
// minute in its colour. A tile opens its page.
ResourcePage {
    id: page

    required property var cpu
    required property var memory
    required property var health
    required property var devices
    required property var gpu
    required property var battery
    // The sidebar's icons (Main.qml).
    required property var icons

    // A tile asks for its page ("cpu", "disk:nvme0n1").
    signal openPage(string name)

    // Until the first tick, say nothing rather than "all is well".
    readonly property bool measured: page.memory.total > 0
    readonly property color background: Kirigami.Theme.backgroundColor
    // The samples in a history (series.rs's LEN): the lists of several
    // devices come one after another, this many each.
    readonly property int span: 60

    function percent(v) {
        return isNaN(v) ? "" : Math.round(v) + "%";
    }

    function trend(list, index) {
        return list.slice(index * page.span, (index + 1) * page.span);
    }

    title: qsTr("Overview")
    headline: qsTr("Overview")
    headlineScale: 2.1

    // The worst problem, or that there is none, and the one thing to do
    // about it.
    RowLayout {
        Layout.fillWidth: true
        visible: page.measured
        spacing: Kirigami.Units.largeSpacing

        Kirigami.Icon {
            Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
            Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
            source: page.health.level === 2 ? "dialog-error" : page.health.level === 1 ? "dialog-warning" : "checkmark"
            isMask: page.health.level === 0
            color: Kirigami.Theme.positiveTextColor
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 0

            QQC2.Label {
                Layout.fillWidth: true
                text: page.health.level === 0 ? qsTr("Everything looks fine") : page.health.titles[0] ?? ""
                font.weight: Font.DemiBold
                wrapMode: Text.Wrap
                textFormat: Text.PlainText
            }
            QQC2.Label {
                Layout.fillWidth: true
                text: page.health.level === 0 ? qsTr("Nothing on this computer needs your attention.") : page.health.details[0] ?? ""
                opacity: 0.7
                wrapMode: Text.Wrap
                textFormat: Text.PlainText
            }
        }
        SecondaryButton {
            readonly property string target: page.health.level > 0 ? (page.health.pages[0] ?? "") : ""
            visible: target.length > 0
            text: target === "services" ? qsTr("Open Services") : qsTr("Open Apps")
            onClicked: page.openPage(target)
        }
    }

    GridLayout {
        id: tiles
        readonly property real tileWidth: Kirigami.Units.gridUnit * 15
        Layout.fillWidth: true
        Layout.topMargin: Kirigami.Units.smallSpacing
        columns: Math.max(1, Math.floor((width + columnSpacing) / (tileWidth + columnSpacing)))
        columnSpacing: Kirigami.Units.largeSpacing
        rowSpacing: Kirigami.Units.largeSpacing

        OverviewTile {
            Layout.fillWidth: true
            text: qsTr("Processor")
            hue: Hues.on(Hues.cpu, page.background)
            values: page.devices.cpuTrend
            value: page.measured ? Format.percent(page.cpu.usage) : ""
            detail: isNaN(page.cpu.temperature) ? "" : Format.celsius(page.cpu.temperature)
            onClicked: page.openPage("cpu")
        }
        OverviewTile {
            Layout.fillWidth: true
            text: qsTr("Memory")
            hue: Hues.on(Hues.memory, page.background)
            values: page.devices.memoryTrend
            value: page.measured ? Format.share(page.memory.used, page.memory.total) : ""
            detail: page.measured ? page.percent(page.memory.used / page.memory.total * 100) : ""
            onClicked: page.openPage("memory")
        }
        Repeater {
            model: page.devices.diskNames.length
            OverviewTile {
                required property int index
                Layout.fillWidth: true
                text: page.devices.diskLabels[index] ?? ""
                hue: Hues.on(Hues.disk, page.background)
                values: page.trend(page.devices.diskTrends, index)
                maximum: 0
                // A quiet drive's chart still spans a megabyte a second.
                minimumScale: 1048576
                value: Format.rate(page.devices.diskRates[index] ?? NaN)
                detail: page.devices.diskNames[index] ?? ""
                onClicked: page.openPage("disk:" + page.devices.diskNames[index])
            }
        }
        Repeater {
            model: page.devices.netNames.length
            OverviewTile {
                required property int index
                Layout.fillWidth: true
                text: page.devices.netLabels[index] ?? ""
                hue: Hues.on(Hues.network, page.background)
                values: page.trend(page.devices.netTrends, index)
                maximum: 0
                minimumScale: 102400
                value: Format.rate(page.devices.netRates[index] ?? NaN)
                detail: page.devices.netNames[index] ?? ""
                onClicked: page.openPage("network:" + page.devices.netNames[index])
            }
        }
        Repeater {
            model: page.gpu.cardNames.length
            OverviewTile {
                required property int index
                readonly property real temperature: page.gpu.cardTemperatures[index] ?? NaN
                Layout.fillWidth: true
                text: page.gpu.cardLabels[index] ?? ""
                hue: Hues.on(Hues.gpu, page.background)
                values: page.trend(page.devices.gpuTrends, index)
                value: Format.percent(page.gpu.cardUsages[index] ?? NaN)
                detail: isNaN(temperature) ? "" : Format.celsius(temperature)
                onClicked: page.openPage("gpu:" + page.gpu.cardNames[index])
            }
        }
        // The batteries together (the first entry is the total); a group's
        // packs are a click away.
        OverviewTile {
            Layout.fillWidth: true
            visible: page.battery.packNames.length > 0
            text: qsTr("Battery")
            chart: false
            value: Format.percent(page.battery.packPercents[0] ?? NaN)
            onClicked: page.openPage("battery:" + page.battery.packNames[0])
        }
    }

    // With more than one thing wrong, the rest are listed.
    Section {
        visible: page.health.titles.length > 1
        title: qsTr("Also Worth a Look")
        Repeater {
            model: page.health.titles.length > 1 ? page.health.titles.length - 1 : 0
            SectionRow {
                required property int index
                iconName: page.health.levels[index + 1] === 2 ? "dialog-error" : "dialog-warning"
                // The lists change one after another: a new row can read
                // the next list before it has its entry.
                title: page.health.titles[index + 1] ?? ""
                subtitle: page.health.details[index + 1] ?? ""
            }
        }
    }
}
