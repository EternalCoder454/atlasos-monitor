import QtQuick
import QtQuick.Layouts
import QtQuick.Templates as T
import org.kde.kirigami as Kirigami

// A part of a resource page's name ("Health", "Details"), small and muted
// as the Go version's. A `foldable` one has a chevron and folds the part
// under it: the page keeps `folded` and answers `toggled`.
T.AbstractButton {
    id: control

    property bool foldable: false
    property bool folded: false

    // As wide as its words: a click beside them does nothing.
    Layout.topMargin: Kirigami.Units.smallSpacing
    implicitWidth: row.implicitWidth
    implicitHeight: row.implicitHeight
    // Not `enabled: foldable`: a disabled label takes the theme's
    // disabled colours, too faint.
    focusPolicy: foldable ? Qt.StrongFocus : Qt.NoFocus
    hoverEnabled: foldable
    Accessible.role: foldable ? Accessible.Button : Accessible.Heading
    Accessible.name: control.text
    Accessible.description: foldable ? (folded ? qsTr("Collapsed") : qsTr("Expanded")) : ""

    contentItem: Item {}

    RowLayout {
        id: row
        spacing: Kirigami.Units.smallSpacing

        Kirigami.Icon {
            visible: control.foldable
            Layout.preferredWidth: Kirigami.Units.iconSizes.small
            Layout.preferredHeight: Kirigami.Units.iconSizes.small
            source: control.mirrored ? "arrow-left" : "arrow-right"
            isMask: true
            color: Kirigami.Theme.textColor
            opacity: control.hovered ? 0.8 : 0.55
            rotation: control.folded ? 0 : (control.mirrored ? -90 : 90)
            Behavior on rotation {
                NumberAnimation {
                    duration: Kirigami.Units.shortDuration
                }
            }
        }
        Text {
            text: control.text
            color: Kirigami.Theme.textColor
            opacity: control.hovered ? 0.85 : 0.62
            font.family: Kirigami.Theme.defaultFont.family
            font.pointSize: Kirigami.Theme.defaultFont.pointSize * 0.92
            textFormat: Text.PlainText
        }
    }

    // Focus from the keyboard shows as an underline.
    Rectangle {
        visible: control.visualFocus
        anchors.top: row.bottom
        width: row.implicitWidth
        height: 2
        color: Kirigami.Theme.highlightColor
    }
}
