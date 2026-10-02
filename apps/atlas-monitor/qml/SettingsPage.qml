pragma ComponentBehavior: Bound

import QtQuick
import Atlas.Ui

AtlasPage {
    id: page

    required property var backend

    title: qsTr("Settings")

    Component.onCompleted: backend.refreshOwnMemory()

    Section {
        title: qsTr("Refresh")
        footer: qsTr("Only the page on screen is refreshed. Longer intervals use less power.")

        Repeater {
            model: [500, 1000, 2000, 5000]
            delegate: SectionRow {
                required property int modelData
                title: modelData < 1000 ? qsTr("Every half second") : modelData === 1000 ? qsTr("Every second") : qsTr("Every %1 seconds").arg(modelData / 1000)
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
        }
    }
}
