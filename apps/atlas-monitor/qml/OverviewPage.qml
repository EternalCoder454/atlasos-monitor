pragma ComponentBehavior: Bound

import QtQuick
import org.kde.kirigami as Kirigami
import Atlas.Ui

// What is wrong with the machine, then a row per part that opens its page.
AtlasPage {
    id: page

    required property var cpu
    required property var memory
    required property var health
    required property var devices
    required property var gpu
    required property var battery

    // A row asks for its page ("cpu", "disk:nvme0n1").
    signal openPage(string name)

    // Until the first tick, say nothing rather than "all is well".
    readonly property bool measured: page.memory.total > 0

    function percent(v) {
        return Math.round(v) + "%";
    }

    title: qsTr("Overview")

    StatusHero {
        iconName: page.health.level === 0 ? "checkmark" : "dialog-warning"
        tint: page.health.level === 2 ? Kirigami.Theme.negativeTextColor : page.health.level === 1 ? Kirigami.Theme.neutralTextColor : Kirigami.Theme.positiveTextColor
        headline: !page.measured ? "" : page.health.level === 0 ? qsTr("Everything Looks Fine") : page.health.titles[0]
        subtitle: !page.measured ? "" : page.health.level === 0 ? qsTr("Nothing on this computer needs your attention.") : page.health.details[0]
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
                title: page.health.titles[index + 1]
                subtitle: page.health.details[index + 1]
            }
        }
    }

    Section {
        SectionRow {
            iconName: "cpu"
            chevron: true
            onClicked: page.openPage("cpu")
            title: qsTr("Processor")
            subtitle: isNaN(page.cpu.temperature) ? "" : qsTr("%1 °C").arg(Math.round(page.cpu.temperature))
            value: page.measured ? page.percent(page.cpu.usage) : ""
        }
        SectionRow {
            iconName: "memory"
            chevron: true
            onClicked: page.openPage("memory")
            title: qsTr("Memory")
            subtitle: page.measured ? Format.share(page.memory.used, page.memory.total) : ""
            value: page.measured ? page.percent(page.memory.used / page.memory.total * 100) : ""
        }
        Repeater {
            model: page.devices.diskNames.length
            SectionRow {
                required property int index
                iconName: "drive-harddisk"
                chevron: true
                onClicked: page.openPage("disk:" + page.devices.diskNames[index])
                title: page.devices.diskLabels[index] ?? ""
                value: Format.rate(page.devices.diskRates[index] ?? NaN)
            }
        }
        Repeater {
            model: page.devices.netNames.length
            SectionRow {
                required property int index
                iconName: "network-wired"
                chevron: true
                onClicked: page.openPage("network:" + page.devices.netNames[index])
                title: page.devices.netLabels[index] ?? ""
                value: Format.rate(page.devices.netRates[index] ?? NaN)
            }
        }
        Repeater {
            model: page.gpu.cardNames.length
            SectionRow {
                required property int index
                readonly property real temperature: page.gpu.cardTemperatures[index] ?? NaN
                iconName: "show-gpu-effects"
                chevron: true
                onClicked: page.openPage("gpu:" + page.gpu.cardNames[index])
                title: page.gpu.cardLabels[index] ?? ""
                subtitle: isNaN(temperature) ? "" : qsTr("%1 °C").arg(Math.round(temperature))
                value: Format.percent(page.gpu.cardUsages[index] ?? NaN)
            }
        }
        // The batteries together; a group's packs are a click away.
        SectionRow {
            visible: page.battery.packNames.length > 0
            iconName: "battery"
            chevron: true
            onClicked: page.openPage("battery:" + page.battery.packNames[0])
            title: qsTr("Battery")
            value: Format.percent(page.battery.packPercents[0] ?? NaN)
        }
    }
}
