import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import QtQuick.Templates as T
import org.kde.kirigami as Kirigami
import Atlas.Ui

// One part of the computer on the Overview, as Task Manager's performance
// list: a small chart of its last minute in its colour, its name, its
// figure now and a detail. A click opens its page.
T.AbstractButton {
    id: tile

    property string value
    property string detail
    property color hue
    property list<real> values
    // The value at the chart's top; 0 scales to the samples, at least
    // minimumScale (rates).
    property real maximum: 100
    property real minimumScale: 0
    // Off for a part with no history to draw (the battery).
    property bool chart: true

    // The chart takes the samples only when they differ: the Overview
    // slices each device's history out of one list for all of them, so one
    // busy disk would otherwise repaint every idle disk's flat line.
    function same(a, b) {
        if (a.length !== b.length) {
            return false;
        }
        for (let i = 0; i < a.length; ++i) {
            if (a[i] !== b[i] && !(isNaN(a[i]) && isNaN(b[i]))) {
                return false;
            }
        }
        return true;
    }
    onValuesChanged: {
        if (!tile.same(tile.values, chart.values)) {
            chart.values = tile.values;
        }
    }
    Component.onCompleted: chart.values = tile.values

    implicitWidth: Kirigami.Units.gridUnit * 15
    implicitHeight: Math.max(Kirigami.Units.gridUnit * 3.6, column.implicitHeight) + padding * 2
    padding: Kirigami.Units.largeSpacing
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus
    Accessible.role: Accessible.Button
    Accessible.name: tile.text
    Accessible.description: [tile.value, tile.detail].filter(t => t.length > 0).join(", ")
    Keys.onReturnPressed: event => {
        if (!event.isAutoRepeat) {
            tile.clicked();
        }
    }
    Keys.onEnterPressed: event => {
        if (!event.isAutoRepeat) {
            tile.clicked();
        }
    }

    background: Rectangle {
        radius: 6
        color: Qt.alpha(Kirigami.Theme.textColor, tile.down ? 0.1 : tile.hovered ? 0.065 : 0.035)
        border.width: tile.visualFocus ? 2 : 1
        border.color: tile.visualFocus ? Qt.alpha(Kirigami.Theme.highlightColor, 0.6) : Qt.alpha(Kirigami.Theme.textColor, 0.08)
    }

    contentItem: RowLayout {
        spacing: Kirigami.Units.largeSpacing * 1.5

        LiveChart {
            id: chart
            visible: tile.chart
            Layout.preferredWidth: Kirigami.Units.gridUnit * 5
            Layout.fillHeight: true
            captions: false
            color: tile.hue
            maximum: tile.maximum
            minimumScale: tile.minimumScale
        }
        ColumnLayout {
            id: column
            Layout.fillWidth: true
            spacing: 0

            QQC2.Label {
                Layout.fillWidth: true
                text: tile.text
                font.pointSize: Kirigami.Theme.defaultFont.pointSize * 1.15
                textFormat: Text.PlainText
                elide: Text.ElideRight
            }
            QQC2.Label {
                Layout.fillWidth: true
                text: tile.value
                opacity: 0.75
                font.features: ({
                        "tnum": 1
                    })
                textFormat: Text.PlainText
                elide: Text.ElideRight
            }
            QQC2.Label {
                Layout.fillWidth: true
                visible: tile.detail.length > 0
                text: tile.detail
                opacity: 0.55
                font: Kirigami.Theme.smallFont
                textFormat: Text.PlainText
                elide: Text.ElideRight
            }
        }
    }
}
