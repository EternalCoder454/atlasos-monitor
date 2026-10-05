import QtQuick
import QtQuick.Layouts
import Atlas.Ui

// A part's figures in one card: the few that matter large across the top
// (`stats`, AtlasStats), the rest as a grid of names and values under a line.
//
//   FigureCard {
//       AtlasStat { label: qsTr("Speed"); value: "4.2 GHz" }
//       details: [[qsTr("Cores"), "24"], [qsTr("Sockets"), "1"]]
//   }
AtlasCard {
    id: card

    default property alias stats: statRow.data
    // [[name, value], ...]; a row whose value is "" or undefined is left out.
    property var details: []

    readonly property var shown: card.details.filter(r => r[1] !== undefined && r[1] !== "").map(r => ({
                "label": r[0],
                "value": r[1]
            }))

    // Explicitly, not as children: they would go to `stats` itself.
    content: [
        Flow {
            id: statRow
            Layout.fillWidth: true
            // Not visibleChildren: it sticks at 0.
            visible: statRow.children.length > 0
            spacing: AtlasStyle.spacingXXLarge
        },
        Rectangle {
            Layout.fillWidth: true
            visible: statRow.visible && card.shown.length > 0
            implicitHeight: 1
            color: AtlasStyle.separator
        },
        AtlasDetailGrid {
            Layout.fillWidth: true
            visible: card.shown.length > 0
            // Two to a line where there is room.
            columns: 2
            model: card.shown
        }
    ]
}
