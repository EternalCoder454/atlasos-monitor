import QtQuick
import QtQuick.Layouts
import ChartBench

// A page-like window: a sidebar, then the charts, each in a rounded section
// like Atlas.Ui's Section.
Window {
    id: root

    required property var seriesList
    required property int columns
    required property bool captions
    required property bool useList
    required property bool opaque

    visible: true
    title: "Chart Bench"
    color: "#202326"

    Rectangle {
        id: sidebar
        width: 240
        anchors { top: parent.top; bottom: parent.bottom; left: parent.left }
        color: "#292c30"
        Column {
            anchors { fill: parent; margins: 12 }
            spacing: 4
            Repeater {
                model: ["Overview", "CPU", "Memory", "Disk", "Network", "GPU", "Sensors", "Apps", "Services"]
                Rectangle {
                    width: parent.width; height: 32; radius: 8
                    color: index === 1 ? "#3daee9" : "transparent"
                    Text { anchors.verticalCenter: parent.verticalCenter; x: 12; text: modelData; color: "#fcfcfc" }
                }
            }
        }
    }

    GridLayout {
        anchors { top: parent.top; bottom: parent.bottom; left: sidebar.right; right: parent.right; margins: 20 }
        columns: root.columns
        rowSpacing: 12
        columnSpacing: 12

        Repeater {
            model: root.seriesList
            Rectangle {
                id: section
                required property var modelData
                required property int index
                Layout.fillWidth: true
                Layout.fillHeight: true
                radius: 10
                color: Qt.tint("#202326", Qt.rgba(1, 1, 1, 0.06))
                border.width: 1
                border.color: Qt.rgba(0.99, 0.99, 0.99, 0.12)

                LiveChart {
                    anchors { fill: parent; margins: 10 }
                    series: root.useList ? null : modelData
                    values: root.useList ? modelData.values : []
                    label: ["CPU", "Memory", "Download", "Upload", "Read", "Write", "GPU", "VRAM"][index % 8]
                    color: ["#39b8e3", "#5c9efa", "#f5628e", "#f5628e", "#84c718", "#84c718", "#de68f2", "#de68f2"][index % 8]
                    percent: index % 2 === 0
                    captions: root.captions
                    background: root.opaque ? section.color : "transparent"
                }
            }
        }
    }
}
