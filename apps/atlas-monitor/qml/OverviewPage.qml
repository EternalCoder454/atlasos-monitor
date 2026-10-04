pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

// The whole computer at a glance: what, if anything, is wrong, then a row
// per part with its figure and its last minute as a sparkline in its
// colour. A row opens its page.
AtlasPage {
    id: page

    required property var cpu
    required property var memory
    required property var health
    required property var devices
    required property var gpu
    required property var battery
    // The sidebar's icons (Main.qml).
    required property var icons

    // A row asks for its page ("cpu", "disk:nvme0n1").
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

    // The last minute beside a row's figure.
    component Trend: Sparkline {
        width: Kirigami.Units.gridUnit * 4.5
        height: Kirigami.Units.gridUnit * 1.4
        anchors.verticalCenter: parent?.verticalCenter
        Accessible.ignored: true
    }

    title: qsTr("Overview")

    // The worst problem, or that there is none, and the one thing to do
    // about it.
    Section {
        visible: page.measured

        SectionRow {
            iconName: page.health.level === 2 ? "dialog-error" : page.health.level === 1 ? "dialog-warning" : "checkmark"
            title: page.health.level === 0 ? qsTr("Everything looks fine") : page.health.titles[0] ?? ""
            subtitle: page.health.level === 0 ? qsTr("Nothing on this computer needs your attention.") : page.health.details[0] ?? ""

            SecondaryButton {
                readonly property string target: page.health.level > 0 ? (page.health.pages[0] ?? "") : ""
                visible: target.length > 0
                text: target === "services" ? qsTr("Open Services") : qsTr("Open Apps")
                onClicked: page.openPage(target)
            }
        }
    }

    Section {
        title: qsTr("Hardware")

        SectionRow {
            iconName: page.icons.cpu
            chevron: true
            onClicked: page.openPage("cpu")
            title: qsTr("Processor")
            subtitle: isNaN(page.cpu.temperature) ? "" : Format.celsius(page.cpu.temperature)
            value: page.measured ? page.percent(page.cpu.usage) : ""

            Trend {
                color: Hues.on(Hues.cpu, page.background)
                values: page.devices.cpuTrend
            }
        }
        SectionRow {
            iconName: page.icons.memory
            chevron: true
            onClicked: page.openPage("memory")
            title: qsTr("Memory")
            subtitle: page.measured ? Format.share(page.memory.used, page.memory.total) : ""
            value: page.measured ? page.percent(page.memory.used / page.memory.total * 100) : ""

            Trend {
                color: Hues.on(Hues.memory, page.background)
                values: page.devices.memoryTrend
            }
        }
        Repeater {
            model: page.devices.diskNames.length
            SectionRow {
                id: diskRow
                required property int index
                iconName: page.icons.disk
                chevron: true
                onClicked: page.openPage("disk:" + page.devices.diskNames[index])
                // The lists change one after another: a new row can read
                // the next list before it has its entry.
                title: page.devices.diskLabels[index] ?? ""
                subtitle: page.devices.diskNames[index] ?? ""
                value: Format.rate(page.devices.diskRates[index] ?? NaN)

                Trend {
                    color: Hues.on(Hues.disk, page.background)
                    values: page.trend(page.devices.diskTrends, diskRow.index)
                    maximum: 0
                    // A quiet drive's line still spans a megabyte a second.
                    minimumScale: 1048576
                }
            }
        }
        Repeater {
            model: page.devices.netNames.length
            SectionRow {
                id: netRow
                required property int index
                iconName: page.icons.wired
                chevron: true
                onClicked: page.openPage("network:" + page.devices.netNames[index])
                title: page.devices.netLabels[index] ?? ""
                subtitle: page.devices.netNames[index] ?? ""
                value: Format.rate(page.devices.netRates[index] ?? NaN)

                Trend {
                    color: Hues.on(Hues.network, page.background)
                    values: page.trend(page.devices.netTrends, netRow.index)
                    maximum: 0
                    minimumScale: 102400
                }
            }
        }
        Repeater {
            model: page.gpu.cardNames.length
            SectionRow {
                id: gpuRow
                required property int index
                readonly property real temperature: page.gpu.cardTemperatures[index] ?? NaN
                iconName: page.icons.gpu
                chevron: true
                onClicked: page.openPage("gpu:" + page.gpu.cardNames[index])
                title: page.gpu.cardLabels[index] ?? ""
                subtitle: isNaN(temperature) ? "" : Format.celsius(temperature)
                value: Format.percent(page.gpu.cardUsages[index] ?? NaN)

                Trend {
                    color: Hues.on(Hues.gpu, page.background)
                    values: page.trend(page.devices.gpuTrends, gpuRow.index)
                }
            }
        }
        // The batteries together (the first entry is the total); a group's
        // packs are a click away.
        SectionRow {
            visible: page.battery.packNames.length > 0
            iconName: page.icons.battery
            chevron: true
            onClicked: page.openPage("battery:" + page.battery.packNames[0])
            title: qsTr("Battery")
            value: Format.percent(page.battery.packPercents[0] ?? NaN)
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
                title: page.health.titles[index + 1] ?? ""
                subtitle: page.health.details[index + 1] ?? ""
            }
        }
    }
}
