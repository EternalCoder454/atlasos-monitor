import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

// A Section card with room inside, for a chart, figures or a grid rather
// than rows: the same title, fold and card as every Atlas page.
//
//   MonitorCard {
//       title: qsTr("Swap")
//       LiveChart { Layout.fillWidth: true }
//   }
Section {
    id: card

    default property alias body: inner.data

    content: ColumnLayout {
        id: inner
        Layout.fillWidth: true
        Layout.margins: Kirigami.Units.largeSpacing * 1.5
        spacing: Kirigami.Units.largeSpacing * 1.5
    }
}
