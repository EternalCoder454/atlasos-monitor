import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Telamon.Ui

TelamonPage {
    id: page

    required property var backend

    title: qsTr("About")

    ColumnLayout {
        Layout.fillWidth: true
        Layout.topMargin: Kirigami.Units.gridUnit
        spacing: Kirigami.Units.smallSpacing

        Kirigami.Icon {
            Layout.alignment: Qt.AlignHCenter
            source: "net.eterneon.atlas.monitor"
            Layout.preferredWidth: Math.round(Kirigami.Units.gridUnit * 5)
            Layout.preferredHeight: Layout.preferredWidth
        }
        TelamonLabel {
            Layout.alignment: Qt.AlignHCenter
            Layout.topMargin: Kirigami.Units.smallSpacing
            textStyle: TelamonLabel.Title
            text: qsTr("Atlas Monitor")
        }
        QQC2.Label {
            Layout.alignment: Qt.AlignHCenter
            opacity: 0.7
            text: qsTr("Version %1").arg(Qt.application.version)
        }
        QQC2.Label {
            Layout.fillWidth: true
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.Wrap
            opacity: 0.7
            text: qsTr("See what your computer is doing and what is slowing it down.")
        }
    }

    Section {
        title: qsTr("About")
        footer: qsTr("Atlas Monitor collects nothing. Crash reports are off unless you turn them on in Atlas Updater, and each report is shown to you before it's sent.")
        SectionRow {
            title: qsTr("License")
            value: qsTr("MIT")
        }
        SectionRow {
            title: qsTr("Made by")
            value: qsTr("Eterneon")
        }
        SectionRow {
            title: qsTr("Project Page")
            chevron: true
            onClicked: Qt.openUrlExternally("https://github.com/EternalCoder454/atlasos-monitor")
        }
    }
}
