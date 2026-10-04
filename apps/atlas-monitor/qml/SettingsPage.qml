pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

AtlasPage {
    id: page

    required property var backend
    required property var energy

    // Release Idle Memory: the window does it, so leaving the page at once
    // doesn't stop it.
    signal releaseIdleMemory

    title: qsTr("Settings")

    Component.onCompleted: backend.refreshOwnMemory()

    Section {
        title: qsTr("Refresh")
        footer: qsTr("Only the page on screen is refreshed. Longer intervals use less power.")

        Repeater {
            // settings.rs INTERVALS_MS
            model: [500, 1000, 2000, 3000, 5000, 10000]
            delegate: SectionRow {
                required property int modelData
                title: modelData < 1000 ? qsTr("Every half second") : modelData === 1000 ? qsTr("Every second") : qsTr("Every %1 seconds").arg(modelData / 1000)
                subtitle: qsTr("Charts cover the last %1").arg(Format.span(modelData))
                clickable: true
                radio: true
                checkmark: page.backend.refreshInterval === modelData
                Accessible.role: Accessible.RadioButton
                Accessible.checked: checkmark
                onClicked: page.backend.changeRefreshInterval(modelData)
            }
        }
    }

    Section {
        title: qsTr("Energy Saver")
        footer: page.energy.unavailable.length > 0 ? qsTr("Energy Saver can't work on this system; its page says why.") : qsTr("An app that keeps a core busy for half a minute is put behind the rest, and put back when it calms down. Apps playing or recording sound, the app you're using, and terminals are left alone.")

        SectionRow {
            title: qsTr("Ease Off Busy Apps Automatically")
            showSwitch: true
            switchChecked: page.energy.automatic
            enabled: page.energy.unavailable.length === 0
            onSwitchToggled: checked => page.energy.setAutomaticEasing(checked)
        }
    }

    Section {
        title: qsTr("Drawing")
        footer: qsTr("Atlas Monitor draws its window with the processor, which uses less memory. Takes effect the next time Atlas Monitor opens.")

        SectionRow {
            title: qsTr("Use the graphics card to draw the window")
            showSwitch: true
            switchChecked: page.backend.gpuRendering
            onSwitchToggled: checked => page.backend.changeGpuRendering(checked)
        }
    }

    Section {
        title: qsTr("Atlas Monitor")

        SectionRow {
            title: qsTr("Memory used")
            subtitle: qsTr("Shared memory is split between the apps that use it.")
            value: page.backend.ownPss > 0 ? Qt.locale().formattedDataSize(page.backend.ownPss) : ""

            SecondaryButton {
                text: qsTr("Release Idle Memory")
                onClicked: page.releaseIdleMemory()
            }
        }
    }

    Section {
        title: qsTr("Crash Reports")
        footer: qsTr("Crash reports for every Atlas app are turned on or off in Atlas Updater. They're off unless you turn them on, and each one is shown to you before it's sent.")

        SectionRow {
            title: qsTr("Open Atlas Updater")
            chevron: true
            onClicked: missing.shown = !page.backend.openUpdater()
        }
    }

    InfoBanner {
        id: missing
        Layout.fillWidth: true
        type: "warning"
        closable: true
        // Shown by what it reports.
        shown: false
        text: qsTr("Atlas Updater isn't installed.")
    }
}
