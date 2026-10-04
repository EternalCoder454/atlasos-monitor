import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// A hardware page: AtlasPage's bold title, with what the part is under it
// and its figure now at the right, then cards of charts and figures. Wider
// than AtlasPage, as charts want, and tighter, as a monitor shows a lot.
// The pages that are lists and settings keep AtlasPage.
Item {
    id: root

    // Bold at the top left: "Processor", or a drive's name.
    property string title
    // Muted under the title: the model, or the size and device.
    property string subtitle
    // At the right of the title, in the part's colour: "19%". "" for none.
    property string figure
    property color figureColor: Kirigami.Theme.textColor
    default property alias content: col.data

    readonly property real maxContentWidth: Kirigami.Units.gridUnit * 54

    QQC2.ScrollView {
        id: scroll
        anchors.fill: parent
        contentWidth: width
        // As AtlasPage: the scrollbar overlays the content.
        leftPadding: 0
        rightPadding: 0
        topPadding: 0
        bottomPadding: 0
        QQC2.ScrollBar.horizontal.policy: QQC2.ScrollBar.AlwaysOff

        QQC2.ScrollBar.vertical: QQC2.ScrollBar {
            id: bar
            parent: scroll
            x: scroll.mirrored ? 0 : scroll.width - width
            height: scroll.height
            policy: QQC2.ScrollBar.AsNeeded
            implicitWidth: 10
            padding: 2
            contentItem: Rectangle {
                implicitWidth: 6
                radius: width / 2
                color: Qt.alpha(Kirigami.Theme.textColor, bar.pressed ? 0.45 : bar.hovered ? 0.35 : 0.22)
                opacity: bar.active ? 1 : 0
                Behavior on opacity {
                    NumberAnimation {
                        duration: Kirigami.Units.longDuration
                    }
                }
            }
            background: null
        }

        Item {
            width: scroll.width
            implicitHeight: col.implicitHeight + Kirigami.Units.gridUnit * 2.4

            ColumnLayout {
                id: col
                y: Kirigami.Units.gridUnit * 1.2
                width: Math.min(parent.width - Kirigami.Units.gridUnit * 2, root.maxContentWidth)
                x: Math.round((parent.width - width) / 2)
                spacing: Kirigami.Units.largeSpacing * 1.5

                RowLayout {
                    Layout.fillWidth: true
                    Layout.bottomMargin: -Kirigami.Units.smallSpacing
                    spacing: Kirigami.Units.gridUnit

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 0

                        QQC2.Label {
                            Layout.fillWidth: true
                            text: root.title
                            font.pointSize: Kirigami.Theme.defaultFont.pointSize * 1.6
                            font.bold: true
                            textFormat: Text.PlainText
                            elide: Text.ElideRight
                            Accessible.role: Accessible.Heading
                            Accessible.description: root.figure
                        }
                        QQC2.Label {
                            Layout.fillWidth: true
                            visible: root.subtitle.length > 0
                            text: root.subtitle
                            opacity: 0.65
                            textFormat: Text.PlainText
                            elide: Text.ElideRight
                        }
                    }
                    QQC2.Label {
                        Layout.alignment: Qt.AlignTop
                        visible: root.figure.length > 0
                        text: root.figure
                        color: root.figureColor
                        font.pointSize: Kirigami.Theme.defaultFont.pointSize * 1.6
                        font.weight: Font.DemiBold
                        // Figures of one width, so a changing value doesn't jiggle.
                        font.features: ({
                                "tnum": 1
                            })
                        textFormat: Text.PlainText
                        Accessible.ignored: true
                    }
                }
            }
        }
    }

    // As AtlasPage: a ScrollView doesn't follow keyboard focus, so tabbing
    // to a control below the fold scrolls just enough to show it. Not for a
    // click: scrolling under the pointer could drop the click.
    function ensureVisible(item) {
        const flick = scroll.contentItem as Flickable;
        if (!item || !flick || item.focusReason === Qt.MouseFocusReason)
            return;
        for (let p = item.parent; p !== col; p = p.parent) {
            if (!p)
                return;
        }
        const r = item.mapToItem(flick.contentItem, 0, 0, item.width, item.height);
        const top = flick.contentY;
        const bottom = top + flick.height;
        if (r.y >= top && r.y + r.height <= bottom)
            return;
        const margin = Kirigami.Units.largeSpacing;
        const maxY = Math.max(0, flick.contentHeight - flick.height);
        flick.cancelFlick();
        if (r.y < top)
            flick.contentY = Math.max(0, r.y - margin);
        else
            flick.contentY = Math.min(maxY, r.y - margin, r.y + r.height + margin - flick.height);
    }

    Connections {
        target: root.Window.window
        enabled: root.visible
        function onActiveFocusItemChanged() {
            root.ensureVisible(root.Window.activeFocusItem);
        }
    }
}
