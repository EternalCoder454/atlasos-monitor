pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// Names and values in columns, as the Go version's details: a muted name,
// its value beside it, `pairs` of them to a line. A row whose value is ""
// (or undefined) is left out.
//
//   DetailGrid {
//       entries: [[qsTr("Sockets"), "1"], [qsTr("Cores"), "24"]]
//   }
GridLayout {
    id: grid

    // (Not `rows`: GridLayout has one.)
    property var entries: []
    property int pairs: 1

    readonly property var shown: grid.entries.filter(r => r[1] !== undefined && r[1] !== "")

    columns: Math.max(1, grid.pairs) * 2
    columnSpacing: Kirigami.Units.largeSpacing * 2
    rowSpacing: Kirigami.Units.smallSpacing + 2

    Repeater {
        model: grid.shown.length * 2

        QQC2.Label {
            required property int index
            readonly property bool isValue: index % 2 === 1
            // The last value of a line takes what width is left.
            readonly property bool last: Math.floor(index / 2) % grid.pairs === grid.pairs - 1

            Layout.fillWidth: isValue && last
            // The gap between pairs is wider than within one.
            Layout.rightMargin: isValue && !last ? Kirigami.Units.gridUnit * 1.5 : 0
            text: grid.shown[Math.floor(index / 2)]?.[isValue ? 1 : 0] ?? ""
            opacity: isValue ? 1 : 0.62
            font.features: ({
                    "tnum": 1
                })
            textFormat: Text.PlainText
            elide: isValue ? Text.ElideRight : Text.ElideNone
        }
    }
}
