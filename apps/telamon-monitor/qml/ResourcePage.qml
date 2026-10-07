import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Telamon.Ui

// A hardware page: TelamonPage's bold title, with what the part is under it
// and its figure at the right, then cards of charts and figures. Wider than
// a plain TelamonPage, as charts want. The pages that are lists and settings
// use TelamonPage as it is.
TelamonPage {
    id: root

    // `title` is TelamonPage's: bold at the top left ("Processor", or a drive's name).
    // Muted under the title: the model, or the size and device.
    property string subtitle
    // At the right of the title, in the part's colour: "19%". "" for none.
    property string figure
    property color figureColor: Kirigami.Theme.textColor

    maxContentWidth: Kirigami.Units.gridUnit * 54

    headerTrailing: TelamonLabel {
        visible: root.figure.length > 0
        text: root.figure
        textStyle: TelamonLabel.Title
        color: root.figureColor
        font.weight: Font.DemiBold
        // Figures of one width, so a changing value doesn't jiggle.
        font.features: ({
                "tnum": 1
            })
        // TelamonPage's title has no description: the figure says what it is.
        Accessible.name: root.title.length > 0 ? qsTr("%1: %2").arg(root.title).arg(root.figure) : root.figure
    }

    // TelamonPage has no subtitle slot: this is the first item of its column,
    // pulled up under the title. The page's own content follows it.
    TelamonLabel {
        Layout.fillWidth: true
        Layout.topMargin: -Kirigami.Units.gridUnit
        visible: root.subtitle.length > 0
        text: root.subtitle
        color: TelamonStyle.textMuted
        elide: Text.ElideRight
    }
}
