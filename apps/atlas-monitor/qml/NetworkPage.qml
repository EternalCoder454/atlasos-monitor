pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

// One network interface: what it receives and sends, and its addresses.
ResourcePage {
    id: page

    required property var net
    // The refresh interval in ms, for the charts' time caption.
    required property int interval

    readonly property color hue: Hues.on(Hues.network, Kirigami.Theme.backgroundColor)
    readonly property string linkSpeed: page.net.speed <= 0 ? "" : page.net.speed >= 1000 ? qsTr("%1 Gbit/s").arg(Number(page.net.speed / 1000).toLocaleString(Qt.locale(), "f", page.net.speed % 1000 ? 1 : 0)) : qsTr("%1 Mbit/s").arg(page.net.speed)

    title: page.net.label
    headline: page.net.label
    headlineScale: 2.1
    name: [page.net.name, page.linkSpeed].filter(t => t.length > 0).join(" · ")

    QQC2.Label {
        visible: !page.net.present
        Layout.fillWidth: true
        wrapMode: Text.Wrap
        opacity: 0.7
        text: qsTr("This connection is not there right now.")
    }

    LiveChart {
        Layout.fillWidth: true
        Layout.preferredHeight: Kirigami.Units.gridUnit * 8.5
        color: page.hue
        values: page.net.rxHistory
        // An idle link still shows a scale of 100 KiB a second.
        minimumScale: 102400
        label: qsTr("Receive")
        valueText: Format.rate(page.net.rxRate)
        topText: Format.rate(scaleTop)
        spanText: Format.span(page.interval)
    }
    LiveChart {
        Layout.fillWidth: true
        Layout.preferredHeight: Kirigami.Units.gridUnit * 8.5
        color: page.hue
        values: page.net.txHistory
        minimumScale: 102400
        label: qsTr("Send")
        valueText: Format.rate(page.net.txRate)
        topText: Format.rate(scaleTop)
        spanText: Format.span(page.interval)
    }

    Flow {
        Layout.fillWidth: true
        Layout.topMargin: Kirigami.Units.smallSpacing
        spacing: Kirigami.Units.gridUnit * 2

        GridLayout {
            columns: 2
            columnSpacing: Kirigami.Units.gridUnit * 1.5
            rowSpacing: Kirigami.Units.largeSpacing

            BigStat {
                label: qsTr("Receive")
                value: Format.rate(page.net.rxRate)
                rule: page.hue
            }
            BigStat {
                label: qsTr("Send")
                value: Format.rate(page.net.txRate)
                rule: page.hue
                dashed: true
            }
        }

        DetailGrid {
            // No wider than the page: a long address elides.
            width: Math.min(implicitWidth, parent.width)
            entries: [[qsTr("IPv4 address"), page.net.ipv4 || Format.dash], [qsTr("IPv6 address"), page.net.ipv6 || Format.dash], [qsTr("Hardware address"), page.net.mac], [qsTr("Link speed"), page.linkSpeed], [qsTr("Received since startup"), Format.size(page.net.rxTotal)], [qsTr("Sent since startup"), Format.size(page.net.txTotal)], [qsTr("Interface"), page.net.name]]
        }
    }
}
