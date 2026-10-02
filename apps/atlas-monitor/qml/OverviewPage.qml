import QtQuick
import Atlas.Ui

// Placeholder until `health` and the hardware readers land: the real Overview
// is a status hero from `health` plus compact CPU, memory, disk, network and
// GPU rows (docs/DESIGN.md).
AtlasPage {
    id: page

    required property var backend

    title: qsTr("Overview")

    StatusHero {
        iconName: "speedometer"
        headline: qsTr("Atlas Monitor")
        subtitle: qsTr("Charts and the app list are on the way.")
    }
}
