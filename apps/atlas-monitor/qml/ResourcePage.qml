import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// A hardware page as the Go version and Task Manager lay it out: the whole
// width, a large headline at the top left (the load, or the device's name)
// and what it is at the right, then charts and figures. The pages that are
// lists and settings keep Atlas.Ui's AtlasPage.
Item {
    id: root

    // Large and light at the top left: "89%", or a drive's name.
    property string headline
    // A figure is larger than a name.
    property real headlineScale: 2.5
    // Muted at the top right: the model, or the size and device.
    property string name
    // For screen readers and the window: what the page is about.
    property string title: root.name
    default property alias content: col.data

    readonly property real margin: Kirigami.Units.gridUnit

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
            implicitHeight: col.implicitHeight + root.margin * 2

            ColumnLayout {
                id: col
                x: root.margin
                y: root.margin * 0.8
                width: parent.width - root.margin * 2
                spacing: Kirigami.Units.largeSpacing * 1.5

                RowLayout {
                    Layout.fillWidth: true
                    spacing: Kirigami.Units.gridUnit

                    QQC2.Label {
                        Layout.alignment: Qt.AlignBaseline
                        // A long drive name gives way to the size beside it
                        // only so far.
                        Layout.maximumWidth: col.width * 0.7
                        text: root.headline
                        font.pointSize: Kirigami.Theme.defaultFont.pointSize * root.headlineScale
                        font.weight: Font.Light
                        font.features: ({
                                "tnum": 1
                            })
                        textFormat: Text.PlainText
                        elide: Text.ElideRight
                        Accessible.role: Accessible.Heading
                        Accessible.name: root.title
                        Accessible.description: root.headline
                    }
                    QQC2.Label {
                        Layout.alignment: Qt.AlignBaseline
                        Layout.fillWidth: true
                        horizontalAlignment: Text.AlignRight
                        text: root.name
                        font.pointSize: Kirigami.Theme.defaultFont.pointSize * 1.15
                        opacity: 0.85
                        textFormat: Text.PlainText
                        elide: Text.ElideRight
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
