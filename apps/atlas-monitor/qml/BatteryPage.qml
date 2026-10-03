import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

// A battery, or every pack summed: its charge and draw over the last
// minute, what it is doing, and how worn it is.
AtlasPage {
    id: page

    required property var battery
    // The refresh interval in ms, for the charts' time caption.
    required property int interval

    readonly property var b: page.battery
    // The draw chart means nothing on a machine that never reports a rate.
    readonly property bool hasRate: page.b.watts > 0 || page.b.timeLeft > 0

    // What the battery is doing and, when it can be estimated, for how long.
    readonly property string caption: {
        if (!page.b.present) {
            return qsTr("This battery is not there right now.");
        }
        if (page.b.status === "full" || (page.b.percent >= 99 && page.b.onAc)) {
            return qsTr("Fully charged");
        }
        if (page.b.status === "charging") {
            return page.b.timeLeft > 0 ? qsTr("Charging · %1 until full").arg(Format.duration(page.b.timeLeft)) : qsTr("Charging");
        }
        if (page.b.status === "discharging") {
            return page.b.timeLeft > 0 ? qsTr("On battery · %1 remaining").arg(Format.duration(page.b.timeLeft)) : qsTr("On battery");
        }
        if (page.b.onAc) {
            return page.b.chargeLimit > 0 ? qsTr("Plugged in, held at the %1 charge limit").arg(page.b.chargeLimit + "%") : qsTr("Plugged in, not charging");
        }
        return "";
    }

    title: page.b.label

    Section {
        title: qsTr("Charge")
        footer: page.caption

        LiveChart {
            Layout.fillWidth: true
            Layout.preferredHeight: Kirigami.Units.gridUnit * 10
            Layout.margins: Kirigami.Units.largeSpacing
            values: page.b.chargeHistory
            maximum: 100
            label: qsTr("Charged")
            valueText: Format.percent(page.b.percent)
            topText: "100%"
            spanText: Format.span(page.interval)
        }
    }

    Section {
        title: page.b.status === "charging" ? qsTr("Charging Rate") : qsTr("Power Draw")
        visible: page.hasRate

        LiveChart {
            Layout.fillWidth: true
            Layout.preferredHeight: Kirigami.Units.gridUnit * 6
            Layout.margins: Kirigami.Units.largeSpacing
            values: page.b.drawHistory
            // A laptop at idle still shows a scale of 10 W.
            minimumScale: 10
            label: qsTr("Now")
            valueText: Format.watts(page.b.watts)
            topText: Format.watts(scaleTop)
            spanText: Format.span(page.interval)
        }
    }

    Section {
        title: qsTr("Details")

        SectionRow {
            title: page.b.status === "charging" ? qsTr("Time Until Full") : qsTr("Time Remaining")
            value: Format.duration(page.b.timeLeft)
        }
        SectionRow {
            title: qsTr("Energy")
            value: page.b.full > 0 ? qsTr("%1 of %2").arg(Format.wh(page.b.energy)).arg(Format.wh(page.b.full)) : Format.dash
        }
        SectionRow {
            title: qsTr("Battery Health")
            // As the Go version: worn from 70%, heavily worn below.
            value: isNaN(page.b.health) ? Format.dash : [qsTr("%1 of original"), qsTr("%1 of original · worn"), qsTr("%1 of original · heavily worn")][page.b.wear].arg(Format.percent(page.b.health))
        }
        SectionRow {
            title: qsTr("Design Capacity")
            value: Format.wh(page.b.design)
        }
        SectionRow {
            title: qsTr("Charge Cycles")
            value: page.b.cycles > 0 ? Format.count(page.b.cycles) : Format.dash
        }
        SectionRow {
            visible: page.b.chargeLimit > 0
            title: qsTr("Charge Limit")
            value: page.b.chargeLimit + "%"
        }
        SectionRow {
            title: qsTr("Voltage")
            value: page.b.volts > 0 ? qsTr("%1 V").arg(Number(page.b.volts).toLocaleString(Qt.locale(), "f", 2)) : Format.dash
        }
        SectionRow {
            title: qsTr("Power Adapter")
            value: !page.b.hasAdapter ? Format.dash : page.b.onAc ? (page.b.adapters || qsTr("Connected")) : qsTr("Disconnected")
        }
        SectionRow {
            visible: page.b.identity.length > 0
            title: qsTr("Battery")
            value: page.b.identity
        }
    }
}
