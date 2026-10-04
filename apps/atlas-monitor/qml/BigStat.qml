pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// A figure as Task Manager shows it: a small muted name over a large value.
// A `rule` colour draws a line at its start that ties it to a chart, dashed
// for the second of two (Write after Read, Send after Receive).
RowLayout {
    id: root

    property string label
    property string value
    property color rule: "transparent"
    property bool dashed: false

    spacing: Kirigami.Units.smallSpacing + 2
    Accessible.role: Accessible.StaticText
    Accessible.name: root.label
    Accessible.description: root.value

    Item {
        id: line
        visible: root.rule.a > 0
        Layout.fillHeight: true
        implicitWidth: 2

        Rectangle {
            visible: !root.dashed
            anchors.fill: parent
            color: root.rule
        }
        Column {
            visible: root.dashed
            spacing: 3
            Repeater {
                model: root.dashed ? Math.floor((line.height + 3) / 7) : 0
                Rectangle {
                    width: 2
                    height: 4
                    color: root.rule
                }
            }
        }
    }
    ColumnLayout {
        spacing: 0

        QQC2.Label {
            text: root.label
            opacity: 0.62
            textFormat: Text.PlainText
            // The whole figure is one item for a screen reader.
            Accessible.ignored: true
        }
        QQC2.Label {
            text: root.value
            font.pointSize: Kirigami.Theme.defaultFont.pointSize * 1.6
            // Figures of one width, so a changing value doesn't jiggle.
            font.features: ({
                    "tnum": 1
                })
            textFormat: Text.PlainText
            Accessible.ignored: true
        }
    }
}
