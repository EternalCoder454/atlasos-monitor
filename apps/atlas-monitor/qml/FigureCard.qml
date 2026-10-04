import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// A part's figures in one card: the few that matter large across the top
// (`stats`, BigStats), the rest as a grid of names and values under a line.
//
//   FigureCard {
//       BigStat { label: qsTr("Speed"); value: "4.2 GHz" }
//       details: [[qsTr("Cores"), "24"], [qsTr("Sockets"), "1"]]
//   }
MonitorCard {
    id: card

    default property alias stats: statRow.data
    property alias details: grid.entries

    // Explicitly, not as children: they would go to `stats` itself.
    body: [
        Flow {
            id: statRow
            Layout.fillWidth: true
            visible: statRow.children.length > 0
            spacing: Kirigami.Units.gridUnit * 2.2
        },
        Rectangle {
            Layout.fillWidth: true
            visible: statRow.visible && grid.shown.length > 0
            implicitHeight: 1
            color: Qt.alpha(Kirigami.Theme.textColor, 0.1)
        },
        DetailGrid {
            id: grid
            Layout.fillWidth: true
            visible: grid.shown.length > 0
            // Two to a line where there is room.
            pairs: card.width > Kirigami.Units.gridUnit * 34 ? 2 : 1
        }
    ]
}
