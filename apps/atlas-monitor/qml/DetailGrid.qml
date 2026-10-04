pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// Names and values: a muted name, its value beside it, in `pairs` columns
// of equal width that read across (the first two entries side by side). A
// row whose value is "" (or undefined) is left out.
//
//   DetailGrid {
//       Layout.fillWidth: true
//       entries: [[qsTr("Sockets"), "1"], [qsTr("Cores"), "24"]]
//   }
//
// Give it a width (fill it): a name's widest is half its column, so a grid
// sized by its contents would size itself in a loop.
RowLayout {
    id: grid

    property var entries: []
    property int pairs: 1

    readonly property var shown: grid.entries.filter(r => r[1] !== undefined && r[1] !== "")
    readonly property int count: Math.max(1, grid.pairs)

    spacing: Kirigami.Units.gridUnit * 1.5

    Repeater {
        model: grid.count

        GridLayout {
            id: column
            required property int index
            // Every count-th entry, from this column's own.
            readonly property var own: grid.shown.filter((r, i) => i % grid.count === column.index)

            Layout.alignment: Qt.AlignTop
            Layout.fillWidth: true
            Layout.preferredWidth: 1
            Layout.minimumWidth: 0
            columns: 2
            columnSpacing: Kirigami.Units.largeSpacing * 2
            rowSpacing: Kirigami.Units.smallSpacing + 2

            Repeater {
                model: column.own.length * 2

                QQC2.Label {
                    required property int index
                    readonly property bool isValue: index % 2 === 1

                    Layout.fillWidth: isValue
                    Layout.minimumWidth: 0
                    // A long name gives way to its value in a narrow column.
                    Layout.maximumWidth: isValue ? Number.POSITIVE_INFINITY : grid.width / grid.count * 0.5
                    text: column.own[Math.floor(index / 2)]?.[isValue ? 1 : 0] ?? ""
                    opacity: isValue ? 1 : 0.62
                    font.features: ({
                            "tnum": 1
                        })
                    textFormat: Text.PlainText
                    elide: Text.ElideRight
                }
            }
        }
    }
}
