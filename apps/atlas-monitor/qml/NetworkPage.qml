import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

// One network interface: what it receives and sends, and its addresses.
AtlasPage {
    id: page

    required property var net
    // The refresh interval in ms, for the chart's time caption.
    required property int interval

    title: page.net.label

    Section {
        title: qsTr("Activity")
        footer: page.net.present ? "" : qsTr("This connection is not there right now.")

        LiveChart {
            Layout.fillWidth: true
            Layout.preferredHeight: Kirigami.Units.gridUnit * 10
            Layout.margins: Kirigami.Units.largeSpacing
            values: page.net.rxHistory
            values2: page.net.txHistory
            // An idle link still shows a scale of 100 KiB a second.
            minimumScale: 102400
            label: qsTr("Receive")
            valueText: Format.rate(page.net.rxRate) + "   " + qsTr("Send") + " " + Format.rate(page.net.txRate)
            topText: Format.rate(scaleTop)
            spanText: Format.span(page.interval)
        }
    }

    Section {
        title: qsTr("Details")

        SectionRow {
            title: qsTr("IPv4 Address")
            value: page.net.ipv4 || Format.dash
        }
        SectionRow {
            title: qsTr("IPv6 Address")
            value: page.net.ipv6 || Format.dash
        }
        SectionRow {
            visible: page.net.mac.length > 0
            title: qsTr("Hardware Address")
            value: page.net.mac
        }
        SectionRow {
            visible: page.net.speed > 0
            title: qsTr("Link Speed")
            value: page.net.speed >= 1000 ? qsTr("%1 Gbit/s").arg(Number(page.net.speed / 1000).toLocaleString(Qt.locale(), "f", page.net.speed % 1000 ? 1 : 0)) : qsTr("%1 Mbit/s").arg(page.net.speed)
        }
        SectionRow {
            title: qsTr("Received Since Startup")
            value: Format.size(page.net.rxTotal)
        }
        SectionRow {
            title: qsTr("Sent Since Startup")
            value: Format.size(page.net.txTotal)
        }
        SectionRow {
            title: qsTr("Interface")
            value: page.net.name
        }
    }
}
