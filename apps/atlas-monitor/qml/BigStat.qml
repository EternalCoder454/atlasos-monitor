import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// A figure: a small muted name over its value, a size up.
ColumnLayout {
    id: root

    property string label
    property string value

    spacing: 0
    Accessible.role: Accessible.StaticText
    Accessible.name: root.label
    Accessible.description: root.value

    QQC2.Label {
        text: root.label
        font: Kirigami.Theme.smallFont
        opacity: 0.65
        textFormat: Text.PlainText
        // The whole figure is one item for a screen reader.
        Accessible.ignored: true
    }
    QQC2.Label {
        text: root.value
        font.pointSize: Kirigami.Theme.defaultFont.pointSize * 1.3
        font.weight: Font.Medium
        // Figures of one width, so a changing value doesn't jiggle.
        font.features: ({
                "tnum": 1
            })
        textFormat: Text.PlainText
        Accessible.ignored: true
    }
}
