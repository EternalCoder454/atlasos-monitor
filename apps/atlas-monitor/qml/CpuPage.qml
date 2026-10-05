import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

// The processor: its load over the last minute, each logical processor's
// load, how fast and warm it runs now, and what it is (cores, caches).
ResourcePage {
    id: page

    required property var cpu
    // The app's settings object: which parts are folded.
    required property var backend
    // The refresh interval in ms, for the chart's time caption.
    required property int interval

    // Until the first tick, figures show as a dash, not as 0.
    readonly property bool measured: page.cpu.threads > 0
    readonly property color hue: Hues.on(Hues.cpu, Kirigami.Theme.backgroundColor)
    readonly property bool coresFolded: page.backend.foldedSections.includes("cpu.cores")

    title: qsTr("Processor")
    subtitle: page.cpu.model
    figure: page.measured ? Format.percent(page.cpu.usage) : Format.dash
    figureColor: page.hue

    AtlasCard {
        LiveChart {
            Layout.fillWidth: true
            Layout.preferredHeight: Kirigami.Units.gridUnit * 9
            color: page.hue
            values: page.cpu.usageHistory
            maximum: 100
            label: qsTr("Utilization")
            valueText: page.measured ? Format.percent(page.cpu.usage) : ""
            topText: "100%"
            spanText: Format.span(page.interval)
        }
    }

    AtlasCard {
        visible: page.cpu.coreUsage.length > 0
        title: qsTr("Logical Processors")
        headerTrailing: [
            ToolbarButton {
                focusable: true
                icon.name: LayoutMirroring.enabled ? "arrow-left" : "arrow-right"
                text: page.coresFolded ? qsTr("Show logical processors") : qsTr("Hide logical processors")
                // A quarter turn to point down when open, either way round.
                iconRotation: page.coresFolded ? 0 : (LayoutMirroring.enabled ? -90 : 90)
                onClicked: page.backend.setFolded("cpu.cores", !page.coresFolded)
                Accessible.checkable: true
                Accessible.checked: !page.coresFolded
            }
        ]

        // On a 32-thread machine the grid is the page's biggest drawing.
        // Folded, it isn't drawn, and its binding stops reading the loads.
        CoreGrid {
            visible: !page.coresFolded
            Layout.fillWidth: true
            Layout.preferredHeight: implicitHeight
            values: page.coresFolded ? [] : page.cpu.coreUsage
            color: page.hue
            textColor: Kirigami.Theme.textColor
            font: Kirigami.Theme.smallFont
            labelFormat: qsTr("CPU %1")
            minimumCellWidth: Kirigami.Units.gridUnit * 5.5
            columnSpacing: Kirigami.Units.largeSpacing * 1.5
            rowSpacing: Kirigami.Units.largeSpacing
            mirrored: LayoutMirroring.enabled
            Accessible.role: Accessible.Chart
            Accessible.name: qsTr("Each logical processor's load")
        }
    }

    FigureCard {
        AtlasStat {
            label: qsTr("Utilization")
            value: page.measured ? Format.percent(page.cpu.usage) : Format.dash
        }
        AtlasStat {
            label: qsTr("Speed")
            value: Format.mhz(page.cpu.frequency)
        }
        AtlasStat {
            label: qsTr("Temperature")
            value: Format.celsius(page.cpu.temperature)
        }
        details: [[qsTr("Base speed"), Format.mhz(page.cpu.baseFrequency)], [qsTr("Sockets"), page.measured ? String(page.cpu.sockets) : Format.dash], [qsTr("Cores"), page.measured ? String(page.cpu.cores) : Format.dash], [qsTr("Logical processors"), page.measured ? String(page.cpu.threads) : Format.dash], [qsTr("L1 data cache"), Format.bytes(page.cpu.l1d)], [qsTr("L1 instruction cache"), Format.bytes(page.cpu.l1i)], [qsTr("L2 cache"), Format.bytes(page.cpu.l2)], [qsTr("L3 cache"), Format.bytes(page.cpu.l3)]]
    }
}
