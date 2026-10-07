pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

// Every temperature, fan and power reading the hardware offers, grouped by
// what it belongs to. A processor's many per-core temperatures fold behind
// one "Cores" row that names the hottest.
AtlasPage {
    id: page

    required property var sensors

    readonly property var s: page.sensors
    // Translated once: qsTr looks its file up on every call, and the Cores
    // row's figure changes every tick.
    readonly property string hottestFormat: qsTr("Hottest %1")

    // One reading, by its place in the flat lists. A temperature past the
    // hardware's own limit says so in the theme's warning colours.
    component Reading: SectionRow {
        id: reading
        required property var sensors
        required property int at
        readonly property int warmth: reading.sensors.warmths[reading.at] ?? 0
        title: reading.sensors.labels[reading.at] ?? ""
        value: reading.sensors.values[reading.at] ?? ""

        AtlasLabel {
            visible: reading.warmth > 0
            textStyle: AtlasLabel.Caption
            text: reading.warmth > 1 ? qsTr("Critical") : qsTr("Hot")
            color: reading.warmth > 1 ? Kirigami.Theme.negativeTextColor : Kirigami.Theme.neutralTextColor
        }
    }

    title: qsTr("Sensors")

    QQC2.Label {
        Layout.fillWidth: true
        Layout.bottomMargin: Kirigami.Units.largeSpacing
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
            // Where its readings are in the flat lists: those shown open,
            // and those folded behind Cores.
            readonly property var openAt: device.places(false)
            readonly property var foldedAt: device.places(true)
            readonly property bool folds: device.foldedAt.length > 0
            property bool expanded: false
            // A delegate reused for another device starts folded.
            onTitleChanged: expanded = false

            function places(folded) {
                const at = [];
                for (let i = device.first; i < device.first + device.count; ++i) {
                    if ((page.s.folded[i] ?? false) === folded) {
                        at.push(i);
                    }
                }
                return at;
            }

            title: page.s.names[index] ?? ""
            // The kernel's name for it: what a search for the chip finds.
            footer: (page.s.drivers[index] ?? "") + (page.s.asleep[index] ? " · " + qsTr("asleep, so not read") : "")

            // The readings shown open; folded ones are made only while
            // Cores is open, so a collapsed processor costs one row.
            Repeater {
                model: device.openAt
                Reading {
                    required property int modelData
                    sensors: page.s
                    at: modelData
                }
            }
            SectionRow {
                visible: device.folds
                title: qsTr("Cores")
                value: page.s.hottest[device.index] ? page.hottestFormat.arg(page.s.hottest[device.index]) : ""
                chevron: true
                disclosure: true
                expanded: device.expanded
                onClicked: device.expanded = !device.expanded
            }
            Repeater {
                model: device.expanded ? device.foldedAt : []
                Reading {
                    required property int modelData
                    sensors: page.s
                    at: modelData
                }
            }
        }
    }
}
