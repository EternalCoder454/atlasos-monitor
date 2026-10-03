import QtQuick
import org.kde.kirigami as Kirigami
import Atlas.Ui

// What is wrong with the machine, then a row per part. The rows gain charts
// and open their pages as those land (docs/DESIGN.md).
AtlasPage {
    id: page

    required property var cpu
    required property var memory
    required property var health

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
            title: qsTr("Processor")
            subtitle: isNaN(page.cpu.temperature) ? "" : qsTr("%1 °C").arg(Math.round(page.cpu.temperature))
            value: page.measured ? page.percent(page.cpu.usage) : ""
        }
        SectionRow {
            iconName: "memory"
            title: qsTr("Memory")
            subtitle: page.measured ? qsTr("%1 of %2").arg(Qt.locale().formattedDataSize(page.memory.used)).arg(Qt.locale().formattedDataSize(page.memory.total)) : ""
            value: page.measured ? page.percent(page.memory.used / page.memory.total * 100) : ""
        }
    }
}
