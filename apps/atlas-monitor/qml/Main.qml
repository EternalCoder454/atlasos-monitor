pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

QQC2.ApplicationWindow {
    id: root

    // Set from main.cpp through setInitialProperties(); see src/lib.rs.
    required property var backend
    required property var sampler
    required property var cpu
    required property var memory
    required property var health
    required property var devices
    required property var disk
    required property var net
    required property var gpu
    required property var battery
    required property var sensors
    required property var apps
    required property var startup
    required property var services
    required property var details
    required property var energy

    title: qsTr("Atlas Monitor")
    width: Kirigami.Units.gridUnit * 56
    height: Kirigami.Units.gridUnit * 40
    minimumWidth: Kirigami.Units.gridUnit * 26
    minimumHeight: Kirigami.Units.gridUnit * 24
    visible: true
    color: Kirigami.Theme.backgroundColor

    LayoutMirroring.enabled: Qt.application.layoutDirection === Qt.RightToLeft
    LayoutMirroring.childrenInherit: true

    property string currentPage: ""
    // Icons only when the window is narrow.
    readonly property bool compact: width < Kirigami.Units.gridUnit * 40

    readonly property var pages: ({
            "overview": overviewPage,
            "cpu": cpuPage,
            "memory": memoryPage,
            "disk": diskPage,
            "network": networkPage,
            "gpu": gpuPage,
            "battery": batteryPage,
            "sensors": sensorsPage,
            "apps": appsPage,
            "startup": startupPage,
            "services": servicesPage,
            "energy": energyPage,
            "settings": settingsPage,
            "about": aboutPage
        })

    // `name` is a page ("cpu"), or a kind and a device ("disk:nvme0n1").
    function showPage(name) {
        if (name === currentPage) {
            return;
        }
        // Split at the first colon only: an alias interface is "eth0:1".
        const colon = name.indexOf(":");
        const kind = colon < 0 ? name : name.slice(0, colon);
        const device = colon < 0 ? undefined : name.slice(colon + 1);
        const known = pages[kind] !== undefined && (device !== undefined) === ["disk", "network", "gpu", "battery"].includes(kind) && device !== "";
        var c = known ? pages[kind] : overviewPage;
        currentPage = known ? name : "overview";
        // Only the page on screen is sampled.
        sampler.showPage(currentPage);
        // A device page's object forgets the last device first, so the page
        // opens on dashes, not on the previous drive's figures.
        if (known && kind === "disk") {
            disk.show(device, devices.diskLabels[devices.diskNames.indexOf(device)] ?? device);
        } else if (known && kind === "network") {
            net.show(device, devices.netLabels[devices.netNames.indexOf(device)] ?? device);
        } else if (known && kind === "gpu") {
            gpu.show(device, gpu.cardLabels[gpu.cardNames.indexOf(device)] ?? device);
        } else if (known && kind === "battery") {
            battery.show(device, battery.packLabels[battery.packNames.indexOf(device)] ?? device);
        }
        if (stack.depth === 0) {
            stack.push(c, {}, QQC2.StackView.Immediate);
        } else {
            stack.replace(c);
        }
    }

    // A group's name above its entries; hidden when the sidebar is icons only.
    component NavHeading: QQC2.Label {
        Layout.fillWidth: true
        Layout.topMargin: Kirigami.Units.largeSpacing
        Layout.bottomMargin: Kirigami.Units.smallSpacing
        Layout.leftMargin: Kirigami.Units.largeSpacing
        visible: !root.compact
        font: Kirigami.Theme.smallFont
        opacity: 0.6
        elide: Text.ElideRight
    }

    // Disk and Network: a group with an entry and its live rate per device.
    // Graphics cards (no figure) and batteries (percent charged) are a group
    // when there are two or more.
    // Compact, the group is one icon that opens the first device.
    component DeviceGroup: SidebarGroup {
        id: group
        required property string kind
        property list<string> names
        property list<string> labels
        property list<real> rates
        property list<real> percents

        visible: names.length > 0
        compact: root.compact
        onActivated: root.showPage(kind + ":" + names[0])

        Repeater {
            model: group.names.length
            SidebarItem {
                required property int index
                readonly property string page: group.kind + ":" + group.names[index]
                Layout.fillWidth: true
                sub: true
                text: group.labels[index] ?? ""
                value: group.rates.length > 0 ? Format.rate(group.rates[index] ?? NaN) : group.percents.length > 0 ? Format.percent(group.percents[index] ?? NaN) : ""
                selected: root.currentPage === page
                onClicked: root.showPage(page)
            }
        }
    }

    component NavItem: SidebarItem {
        required property string page
        Layout.fillWidth: true
        compact: root.compact
        selected: root.currentPage === page
        QQC2.ToolTip.visible: compact && hovered
        QQC2.ToolTip.text: text
        QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
        onClicked: root.showPage(page)
    }

    RowLayout {
        anchors.fill: parent
        spacing: 0

        Rectangle {
            id: sidebar
            Layout.fillHeight: true
            Layout.preferredWidth: root.compact ? Kirigami.Units.gridUnit * 3.6 : Kirigami.Units.gridUnit * 12.5
            color: Qt.tint(Kirigami.Theme.backgroundColor, Qt.alpha(Kirigami.Theme.highlightColor, 0.07))

            Behavior on Layout.preferredWidth {
                NumberAnimation {
                    duration: Kirigami.Units.shortDuration
                    easing.type: Easing.OutCubic
                }
            }

            Rectangle {
                anchors.right: parent.right
                height: parent.height
                width: 1
                color: Qt.alpha(Kirigami.Theme.textColor, 0.12)
            }

            ColumnLayout {
                anchors.fill: parent
                anchors.margins: Kirigami.Units.largeSpacing
                anchors.rightMargin: Kirigami.Units.largeSpacing + 1
                anchors.topMargin: Kirigami.Units.gridUnit
                spacing: 2

                // Energy Saver, Startup and Services join System with their
                // pages; see docs/DESIGN.md.
                NavItem {
                    page: "overview"
                    text: qsTr("Overview")
                    icon.name: "speedometer"
                }
                NavHeading {
                    text: qsTr("Hardware")
                }
                NavItem {
                    page: "cpu"
                    text: qsTr("Processor")
                    icon.name: "cpu"
                }
                NavItem {
                    page: "memory"
                    text: qsTr("Memory")
                    icon.name: "memory"
                }
                DeviceGroup {
                    kind: "disk"
                    text: qsTr("Disk")
                    iconName: "drive-harddisk"
                    names: root.devices.diskNames
                    labels: root.devices.diskLabels
                    rates: root.devices.diskRates
                }
                DeviceGroup {
                    kind: "network"
                    text: qsTr("Network")
                    iconName: "network-wired"
                    names: root.devices.netNames
                    labels: root.devices.netLabels
                    rates: root.devices.netRates
                }
                // One card is one entry; two or more are a group.
                NavItem {
                    visible: root.gpu.cardNames.length === 1
                    page: "gpu:" + (root.gpu.cardNames[0] ?? "")
                    text: qsTr("Graphics")
                    icon.name: "show-gpu-effects"
                }
                DeviceGroup {
                    visible: names.length > 1
                    kind: "gpu"
                    text: qsTr("Graphics")
                    iconName: "show-gpu-effects"
                    names: root.gpu.cardNames
                    labels: root.gpu.cardLabels
                }
                NavItem {
                    visible: root.battery.packNames.length === 1
                    page: "battery:" + (root.battery.packNames[0] ?? "")
                    text: qsTr("Battery")
                    value: Format.percent(root.battery.packPercents[0] ?? NaN)
                    icon.name: "battery"
                }
                DeviceGroup {
                    visible: names.length > 1
                    kind: "battery"
                    text: qsTr("Battery")
                    iconName: "battery"
                    names: root.battery.packNames
                    labels: root.battery.packLabels
                    percents: root.battery.packPercents
                }
                NavItem {
                    visible: root.sensors.available
                    page: "sensors"
                    text: qsTr("Sensors")
                    icon.name: "temperature-normal"
                }
                NavHeading {
                    text: qsTr("System")
                }
                NavItem {
                    page: "apps"
                    text: qsTr("Apps")
                    icon.name: "view-process-all"
                }
                NavItem {
                    page: "energy"
                    text: qsTr("Energy Saver")
                    icon.name: "preferences-system-power-management"
                }
                NavItem {
                    page: "startup"
                    text: qsTr("Startup")
                    icon.name: "system-run"
                }
                NavItem {
                    page: "services"
                    text: qsTr("Services")
                    icon.name: "preferences-system-services"
                }
                Item {
                    Layout.fillHeight: true
                }
                NavItem {
                    page: "settings"
                    text: qsTr("Settings")
                    icon.name: "configure"
                }
                NavItem {
                    page: "about"
                    text: qsTr("About")
                    icon.name: "help-about"
                }
            }
        }

        QQC2.StackView {
            id: stack
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true

            replaceEnter: Transition {
                ParallelAnimation {
                    NumberAnimation {
                        property: "opacity"
                        from: 0
                        to: 1
                        duration: Kirigami.Units.longDuration
                        easing.type: Easing.OutCubic
                    }
                    NumberAnimation {
                        property: "y"
                        from: Kirigami.Units.gridUnit
                        to: 0
                        duration: Kirigami.Units.longDuration
                        easing.type: Easing.OutCubic
                    }
                }
            }
            replaceExit: Transition {
                NumberAnimation {
                    property: "opacity"
                    from: 1
                    to: 0
                    duration: Kirigami.Units.shortDuration
                }
            }
        }
    }

    Connections {
        target: root.backend
        function onRefreshIntervalChanged() {
            root.sampler.changeInterval(root.backend.refreshInterval);
        }
    }

    Component {
        id: overviewPage
        OverviewPage {
            cpu: root.cpu
            memory: root.memory
            health: root.health
            devices: root.devices
            gpu: root.gpu
            battery: root.battery
            onOpenPage: name => root.showPage(name)
        }
    }
    Component {
        id: servicesPage
        ServicesPage {
            services: root.services
        }
    }
    Component {
        id: energyPage
        EnergyPage {
            energy: root.energy
        }
    }
    Component {
        id: startupPage
        StartupPage {
            startup: root.startup
        }
    }
    Component {
        id: appsPage
        AppsPage {
            apps: root.apps
            sampler: root.sampler
            details: root.details
            hasGpu: root.gpu.cardNames.length > 0
        }
    }
    Component {
        id: cpuPage
        CpuPage {
            cpu: root.cpu
            interval: root.backend.refreshInterval
        }
    }
    Component {
        id: memoryPage
        MemoryPage {
            memory: root.memory
            interval: root.backend.refreshInterval
        }
    }
    Component {
        id: diskPage
        DiskPage {
            disk: root.disk
            interval: root.backend.refreshInterval
        }
    }
    Component {
        id: networkPage
        NetworkPage {
            net: root.net
            interval: root.backend.refreshInterval
        }
    }
    Component {
        id: gpuPage
        GpuPage {
            gpu: root.gpu
            interval: root.backend.refreshInterval
        }
    }
    Component {
        id: batteryPage
        BatteryPage {
            battery: root.battery
            interval: root.backend.refreshInterval
        }
    }
    Component {
        id: sensorsPage
        SensorsPage {
            sensors: root.sensors
        }
    }
    Component {
        id: settingsPage
        SettingsPage {
            backend: root.backend
        }
    }
    Component {
        id: aboutPage
        AboutPage {
            backend: root.backend
        }
    }

    Component.onCompleted: showPage("overview")
}
