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
            "settings": settingsPage,
            "about": aboutPage
        })

    // `name` is a page ("cpu"), or a kind and a device ("disk:nvme0n1").
    function showPage(name) {
        if (name === currentPage) {
            return;
        }
        const [kind, device] = name.split(":");
        const known = pages[kind] !== undefined && (device !== undefined) === (kind === "disk" || kind === "network") && device !== "";
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
    // Compact, the group is one icon that opens the first device.
    component DeviceGroup: SidebarGroup {
        id: group
        required property string kind
        property list<string> names
        property list<string> labels
        property list<real> rates

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
                value: Format.rate(group.rates[index] ?? NaN)
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

                // Disk, Network, GPU, Battery and Sensors join Hardware, and
                // System (Apps, Energy Saver, Startup, Services) comes, with
                // their pages; see docs/DESIGN.md.
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
            onOpenPage: name => root.showPage(name)
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
