pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

// Every temperature, fan and power reading the hardware offers, grouped by
// what it belongs to. A processor's many per-core temperatures fold behind
// one "Cores" row that names the hottest. The whole width, as the Go
// version's and the other hardware pages.
ResourcePage {
    id: page

    required property var sensors

    readonly property var s: page.sensors

    // One reading, by its place in the flat lists. A temperature past the
    // hardware's own limit says so in the theme's warning colours.
    component Reading: SectionRow {
        id: reading
        required property var sensors
        required property int at
        readonly property int warmth: reading.sensors.warmths[reading.at] ?? 0
        title: reading.sensors.labels[reading.at] ?? ""
        value: reading.sensors.values[reading.at] ?? ""

        QQC2.Label {
            visible: reading.warmth > 0
            text: reading.warmth > 1 ? qsTr("Critical") : qsTr("Hot")
            color: reading.warmth > 1 ? Kirigami.Theme.negativeTextColor : Kirigami.Theme.neutralTextColor
            font: Kirigami.Theme.smallFont
        }
    }

    title: qsTr("Sensors")
    headline: qsTr("Sensors")
    headlineScale: 2.1

    QQC2.Label {
        Layout.fillWidth: true
        wrapMode: Text.Wrap
        opacity: 0.7
        text: page.s.names.length > 0 ? qsTr("Temperatures, fans and power, as the hardware reports them.") : qsTr("Reading the sensors…")
    }

    Repeater {
        model: page.s.names.length

        Section {
            id: device
            required property int index
            readonly property int first: page.s.first[index] ?? 0
            readonly property int count: page.s.counts[index] ?? 0
            readonly property bool folds: page.s.folded.slice(first, first + count).includes(true)
            property bool expanded: false
            // A delegate reused for another device starts folded.
            onTitleChanged: expanded = false

            title: page.s.names[index] ?? ""
            // The kernel's name for it: what a search for the chip finds.
            footer: (page.s.drivers[index] ?? "") + (page.s.asleep[index] ? " · " + qsTr("asleep, so not read") : "")

            // The readings shown open; folded ones are made only while
            // Cores is open, so a collapsed processor costs one row.
            Repeater {
                model: device.count
                Reading {
                    required property int index
                    sensors: page.s
                    at: device.first + index
                    visible: !(page.s.folded[at] ?? false)
                }
            }
            SectionRow {
                visible: device.folds
                title: qsTr("Cores")
                value: page.s.hottest[device.index] ? qsTr("Hottest %1").arg(page.s.hottest[device.index]) : ""
                chevron: true
                disclosure: true
                expanded: device.expanded
                onClicked: device.expanded = !device.expanded
            }
            Repeater {
                model: device.expanded ? device.count : 0
                Reading {
                    required property int index
                    sensors: page.s
                    at: device.first + index
                    visible: page.s.folded[at] ?? false
                }
            }
        }
    }
}
