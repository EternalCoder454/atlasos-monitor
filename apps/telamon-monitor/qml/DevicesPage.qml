pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Telamon.Ui

// Everything connected to the computer: input devices, what is plugged in
// over USB, and what is inside on the PCI bus, grouped by class. Drives and
// network adapters have their own pages. Read as the page opens.
TelamonPage {
    id: page

    required property var hardware

    readonly property var h: page.hardware

    // Keyboards, mice and the like first; buttons (power, lid) and the rest
    // (audio jack switches) fold behind one row.
    readonly property int pointing: {
        let n = 0;
        for (let i = 0; i < h.inputKinds.length; ++i) {
            if (h.inputKinds[i] !== "buttons" && h.inputKinds[i] !== "other") {
                ++n;
            }
        }
        return n;
    }
    property bool otherInputs: false

    // The PCI list as runs of one class: the reader sorts it by class.
    readonly property var pciGroups: {
        const groups = [];
        for (let i = 0; i < h.pciClasses.length; ++i) {
            const name = h.pciClasses[i];
            if (groups.length === 0 || groups[groups.length - 1].name !== name) {
                groups.push({
                    name: name,
                    first: i,
                    count: 0
                });
            }
            groups[groups.length - 1].count++;
        }
        return groups;
    }

    function kindName(kind) {
        switch (kind) {
        case "keyboard":
            return qsTr("Keyboard");
        case "mouse":
            return qsTr("Mouse");
        case "touchpad":
            return qsTr("Touchpad");
        case "touchscreen":
            return qsTr("Touchscreen");
        case "tablet":
            return qsTr("Drawing tablet");
        case "joystick":
            return qsTr("Game controller");
        case "buttons":
            return qsTr("Buttons");
        }
        return qsTr("Other");
    }

    function kindIcon(kind) {
        switch (kind) {
        case "keyboard":
            return "input-keyboard";
        case "mouse":
            return "input-mouse";
        case "touchpad":
            return "input-touchpad";
        case "touchscreen":
        case "tablet":
            return "input-tablet";
        case "joystick":
            return "input-gaming";
        }
        return "preferences-desktop-peripherals";
    }

    function busName(bus) {
        switch (bus) {
        case "USB":
            return qsTr("USB");
        case "Bluetooth":
            return qsTr("Bluetooth");
        case "Built-in":
            return qsTr("Built-in");
        case "Virtual":
            return qsTr("Virtual");
        }
        return bus;
    }

    title: qsTr("Devices")

    RowLayout {
        Layout.fillWidth: true
        spacing: Kirigami.Units.largeSpacing

        QQC2.Label {
            Layout.fillWidth: true
            wrapMode: Text.Wrap
            opacity: 0.7
            text: qsTr("Everything connected to this computer, as the system sees it. Drives and network adapters have their own pages.")
        }
        SecondaryButton {
            text: qsTr("Refresh")
            icon.name: "view-refresh"
            enabled: !page.h.loading
            onClicked: page.h.refresh()
        }
    }

    QQC2.Label {
        visible: !page.h.loaded
        opacity: 0.7
        text: qsTr("Reading the devices…")
    }

    Section {
        id: input
        visible: page.h.inputNames.length > 0
        title: qsTr("Input Devices")

        Repeater {
            model: page.h.inputNames.length

            SectionRow {
                required property int index
                readonly property string kind: page.h.inputKinds[index] ?? ""
                readonly property bool main: kind !== "buttons" && kind !== "other"

                visible: main || page.otherInputs
                iconName: page.kindIcon(kind)
                title: page.h.inputNames[index] ?? ""
                subtitle: [page.kindName(kind), page.busName(page.h.inputBuses[index] ?? "")].filter(t => t.length > 0).join(" · ")
            }
        }
        SectionRow {
            visible: page.pointing < page.h.inputNames.length
            title: qsTr("Buttons and Switches")
            value: String(page.h.inputNames.length - page.pointing)
            chevron: true
            disclosure: true
            expanded: page.otherInputs
            onClicked: page.otherInputs = !page.otherInputs
        }
    }

    Section {
        visible: page.h.loaded
        title: qsTr("USB")
        footer: page.h.usbNames.length === 0 ? qsTr("Nothing is plugged in over USB.") : ""

        Repeater {
            model: page.h.usbNames.length

            SectionRow {
                required property int index
                readonly property bool hub: page.h.usbHubs[index] ?? false

                iconName: hub ? "network-wired" : "drive-removable-media-usb"
                title: page.h.usbNames[index] ?? ""
                subtitle: [page.h.usbVendors[index] ?? "", hub ? qsTr("Hub") : "", page.h.usbIds[index] ?? ""].filter(t => t.length > 0).join(" · ")
                value: page.h.usbSpeeds[index] ?? ""
            }
        }
    }

    QQC2.Label {
        visible: page.h.loaded && page.pciGroups.length > 0
        Layout.fillWidth: true
        Layout.topMargin: Kirigami.Units.largeSpacing
        wrapMode: Text.Wrap
        opacity: 0.7
        text: qsTr("Inside the computer, by kind. The value is the driver the system uses for it.")
    }

    Repeater {
        model: page.pciGroups

        Section {
            id: group
            required property var modelData
            title: modelData.name
            foldable: true
            // Bridges are the bus's own plumbing: folded until asked for.
            folded: modelData.name === "Bridge"
            onFoldRequested: fold => group.folded = fold

            Repeater {
                model: group.folded ? 0 : group.modelData.count

                SectionRow {
                    required property int index
                    readonly property int at: group.modelData.first + index

                    title: page.h.pciNames[at] ?? ""
                    subtitle: [page.h.pciVendors[at] ?? "", page.h.pciSlots[at] ?? ""].filter(t => t.length > 0).join(" · ")
                    value: page.h.pciDrivers[at] || qsTr("No driver")
                }
            }
        }
    }

    Component.onCompleted: page.h.refresh()
}
