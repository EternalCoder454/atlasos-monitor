pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

// A battery, or every pack summed: its charge and draw over the last
// minute, what it is doing, and how worn it is.
ResourcePage {
    id: page

    required property var battery
    // The refresh interval in ms, for the charts' time caption.
    required property int interval

    readonly property var b: page.battery
    readonly property bool limited: page.b.chargeLimit > 0
    // What the driver says, as the Go version's Status row.
    readonly property string batteryStatus: ({
            "charging": qsTr("Charging"),
            "discharging": qsTr("Discharging"),
            "notCharging": qsTr("Not charging"),
            "full": qsTr("Full")
        })[page.b.status] ?? qsTr("Unknown")
    // The draw chart means nothing on a machine that never reports a rate.
    readonly property bool hasRate: page.b.watts > 0 || page.b.timeLeft > 0
    readonly property color hue: Hues.on(Hues.battery, Kirigami.Theme.backgroundColor)
    readonly property color powerHue: Hues.on(Hues.power, Kirigami.Theme.backgroundColor)
    readonly property string rateName: page.b.status === "charging" ? qsTr("Charging rate") : qsTr("Power draw")

    // What the battery is doing and, when it can be estimated, for how long.
    readonly property string caption: {
        if (!page.b.present) {
            return qsTr("This battery is not there right now.");
        }
        if (page.b.status === "full" || (page.b.percent >= 99 && page.b.onAc)) {
            return qsTr("Fully charged");
        }
        if (page.b.status === "charging") {
            // With a charge limit, the estimate is to the limit.
            if (page.b.timeLeft <= 0) {
                return qsTr("Charging");
            }
            return page.limited ? qsTr("Charging · %1 until %2").arg(Format.duration(page.b.timeLeft)).arg(page.b.chargeLimit + "%") : qsTr("Charging · %1 until full").arg(Format.duration(page.b.timeLeft));
        }
        if (page.b.status === "discharging") {
            return page.b.timeLeft > 0 ? qsTr("On battery · %1 remaining").arg(Format.duration(page.b.timeLeft)) : qsTr("On battery");
        }
        // Held at the limit only when it is there: below it, a start
        // threshold or the firmware (heat) is holding it.
        if (page.b.onAc && page.limited && page.b.percent >= page.b.chargeLimit - 1) {
            return qsTr("Plugged in, held at the %1 charge limit").arg(page.b.chargeLimit + "%");
        }
        if (page.b.onAc) {
            return qsTr("Plugged in, not charging");
        }
        return page.batteryStatus;
    }

    title: page.b.label
    // What the battery is doing and, when it can be estimated, for how long.
    subtitle: page.caption
    figure: Format.percent(page.b.percent)
    figureColor: page.hue

    AtlasCard {
        LiveChart {
            Layout.fillWidth: true
            Layout.preferredHeight: Kirigami.Units.gridUnit * 9
            color: page.hue
            values: page.b.chargeHistory
            maximum: 100
            label: qsTr("Charge")
            valueText: Format.percent(page.b.percent)
            topText: "100%"
            spanText: Format.span(page.interval)
        }
    }

    AtlasCard {
        visible: page.b.present && page.hasRate
        title: page.rateName

        LiveChart {
            Layout.fillWidth: true
            Layout.preferredHeight: Kirigami.Units.gridUnit * 5.5
            color: page.powerHue
            values: page.b.drawHistory
            // A laptop at idle still shows a scale of 10 W.
            minimumScale: 10
            label: page.rateName
            valueText: Format.watts(page.b.watts)
            topText: Format.watts(scaleTop)
            spanText: Format.span(page.interval)
        }
    }

    FigureCard {
        visible: page.b.present

        AtlasStat {
            label: qsTr("Charge")
            value: Format.percent(page.b.percent)
        }
        AtlasStat {
            label: page.rateName
            value: Format.watts(page.b.watts)
        }
        AtlasStat {
            label: page.b.status === "charging" ? qsTr("Time until full") : qsTr("Time remaining")
            value: Format.duration(page.b.timeLeft)
        }
        // As the Go version: worn from 70%, heavily worn below.
        details: [[qsTr("Status"), page.batteryStatus], [qsTr("Energy"), page.b.full > 0 ? Format.shareFormat.arg(Format.wh(page.b.energy)).arg(Format.wh(page.b.full)) : Format.dash], [qsTr("Battery health"), isNaN(page.b.health) ? Format.dash : [qsTr("%1 of original"), qsTr("%1 of original · worn"), qsTr("%1 of original · heavily worn")][page.b.wear].arg(Format.percent(page.b.health))], [qsTr("Design capacity"), Format.wh(page.b.design)], [qsTr("Charge cycles"), page.b.cycles > 0 ? Format.count(page.b.cycles) : Format.dash], [qsTr("Charge limit"), page.b.chargeLimit > 0 ? page.b.chargeLimit + "%" : ""], [qsTr("Voltage"), page.b.volts > 0 ? qsTr("%1 V").arg(Number(page.b.volts).toLocaleString(Qt.locale(), "f", 2)) : Format.dash], [qsTr("Power adapter"), !page.b.hasAdapter ? Format.dash : page.b.onAc ? (page.b.adapters || qsTr("Connected")) : qsTr("Disconnected")], [qsTr("Battery"), page.b.identity]]
    }
}
