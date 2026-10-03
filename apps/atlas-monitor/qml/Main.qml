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
    required property var themeIcons

    // Each page's icon, all from one set so they match. Dracula (AtlasOS's
    // theme) draws the usual names in a mix of outline, solid and colour, and
    // the ones it also has in colour (system-run, help-about) come out in
    // colour at the sidebar's size, so a theme with this whole solid set gets it.
    // Others get Breeze's monochrome names. Breeze's chip is named for GPU
    // effects, and it has no graphics card, so the GPU gets the display.
    readonly property var icons: {
        const solid = {
            overview: "gpm-monitor",
            cpu: "cpu-frequency-indicator",
            memory: "indicator-sensors-memory",
            disk: "indicator-sensors-disk",
            wired: "knemo-network-idle",
            wireless: "network-wireless-signal-excellent",
            gpu: "indicator-sensors-gpu",
            battery: "battery-good",
            sensors: "indicator-sensors-fan",
            apps: "view-process-all",
            energy: "system-devices-panel",
            startup: "media-playback-start",
            services: "view-process-system",
            settings: "configure",
            about: "hb-activity"
        };
        if (Object.values(solid).every(name => themeIcons.has(name))) {
            return solid;
        }
        return {
            overview: "dashboard-show",
            cpu: "show-gpu-effects-symbolic",
            memory: "media-flash-memory-stick-symbolic",
            disk: "drive-harddisk-symbolic",
            wired: "network-wired-symbolic",
            wireless: "network-wireless-symbolic",
            gpu: "computer-symbolic",
            battery: "battery-good-symbolic",
            sensors: "temperature-normal",
            apps: "view-process-all",
            energy: "battery-profile-powersave-symbolic",
            startup: "system-run-symbolic",
            services: "network-server-symbolic",
            settings: "configure",
            about: "help-about-symbolic"
        };
    }

    title: qsTr("Atlas Monitor")
    // As it was left (backend.rs reads it once); the window's minimum
    // still applies.
    width: root.backend.windowWidth > 0 ? root.backend.windowWidth : Kirigami.Units.gridUnit * 56
    height: root.backend.windowHeight > 0 ? root.backend.windowHeight : Kirigami.Units.gridUnit * 40
    minimumWidth: Kirigami.Units.gridUnit * 26
    minimumHeight: Kirigami.Units.gridUnit * 24
    visible: true
    color: Kirigami.Theme.backgroundColor

    LayoutMirroring.enabled: Qt.application.layoutDirection === Qt.RightToLeft
    LayoutMirroring.childrenInherit: true

    // Under everything, past the window's right and bottom edges. At a
    // fractional scale the last device row or column can be only partly
    // inside the window (1150 px at 1.25x is 1437.5 rows). A full repaint
    // clears it first, but the partial updates main.cpp turns on would draw
    // the window's edge half over nothing, a half-transparent line.
    Rectangle {
        id: edgeFill
        z: -1
        width: parent.width + 1
        height: parent.height + 1
        color: root.color
    }

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

    // Its own keys only: "constructor" is no page.
    function isPage(kind) {
        return Object.prototype.hasOwnProperty.call(pages, kind);
    }

    // `name` is a page ("cpu"), or a kind and a device ("disk:nvme0n1").
    function showPage(name) {
        // A page picked before a device's page came back wins over it.
        pendingPage = "";
        forgetPending.stop();
        if (name === currentPage) {
            return;
        }
        gone.stop();
        // Split at the first colon only: an alias interface is "eth0:1".
        const colon = name.indexOf(":");
        const kind = colon < 0 ? name : name.slice(0, colon);
        const device = colon < 0 ? undefined : name.slice(colon + 1);
        const known = isPage(kind) && (device !== undefined) === ["disk", "network", "gpu", "battery"].includes(kind) && device !== "";
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
    // Graphics cards (load) and batteries (percent charged) are a group
    // when there are two or more.
    // Compact, the group is one icon that opens the first device.
    component DeviceGroup: SidebarGroup {
        id: group
        required property string kind
        property list<string> names
        property list<string> labels
        property list<real> rates
        property list<real> percents
        // Percent, blank where unread (a card left to sleep).
        property list<real> loads

        visible: names.length > 0
        compact: root.compact
        onActivated: root.showPage(kind + ":" + names[0])

        Repeater {
            // None while hidden: a lone card's group still has its name,
            // and the row would format its load every tick for nothing.
            model: group.visible ? group.names.length : 0
            SidebarItem {
                required property int index
                readonly property string page: group.kind + ":" + group.names[index]
                Layout.fillWidth: true
                sub: true
                text: group.labels[index] ?? ""
                value: group.rates.length > 0 ? Format.rate(group.rates[index] ?? NaN) : group.percents.length > 0 ? Format.percent(group.percents[index] ?? NaN) : root.load(group.loads[index] ?? NaN)
                selected: root.currentPage === page
                onClicked: root.showPage(page)
            }
        }
    }

    // A sidebar load: blank, not a dash, before the first reading and for
    // a card left unread so it can sleep.
    function load(v) {
        return isNaN(v) ? "" : Format.percent(v);
    }

    component NavItem: SidebarItem {
        required property string page
        Layout.fillWidth: true
        compact: root.compact
        selected: root.currentPage === page
        QQC2.ToolTip.visible: compact && hovered
        QQC2.ToolTip.text: badge.length > 0 ? qsTr("%1 · %2").arg(text).arg(badgeText) : text
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
                // The badge stands in for Go's title-bar warning button: the
                // Overview's list is a click away from any page.
                NavItem {
                    page: "overview"
                    text: qsTr("Overview")
                    icon.name: root.icons.overview
                    badge: root.health.level === 2 ? "dialog-error" : root.health.level === 1 ? "dialog-warning" : ""
                    badgeText: root.health.titles.length === 1 ? root.health.titles[0] : root.health.titles.length > 1 ? qsTr("%1 things need attention").arg(root.health.titles.length) : qsTr("Something needs attention")
                }
                NavHeading {
                    text: qsTr("Hardware")
                }
                NavItem {
                    page: "cpu"
                    text: qsTr("Processor")
                    value: root.load(root.devices.cpuUsage)
                    icon.name: root.icons.cpu
                }
                NavItem {
                    page: "memory"
                    text: qsTr("Memory")
                    value: root.load(root.devices.memoryUsage)
                    icon.name: root.icons.memory
                }
                DeviceGroup {
                    kind: "disk"
                    text: qsTr("Disk")
                    iconName: root.icons.disk
                    names: root.devices.diskNames
                    labels: root.devices.diskLabels
                    rates: root.devices.diskRates
                }
                DeviceGroup {
                    kind: "network"
                    text: qsTr("Network")
                    iconName: root.devices.routeWireless ? root.icons.wireless : root.icons.wired
                    names: root.devices.netNames
                    labels: root.devices.netLabels
                    rates: root.devices.netRates
                }
                // One card is one entry; two or more are a group.
                NavItem {
                    visible: root.gpu.cardNames.length === 1
                    page: "gpu:" + (root.gpu.cardNames[0] ?? "")
                    text: qsTr("Graphics")
                    value: root.load(root.devices.gpuUsages[0] ?? NaN)
                    icon.name: root.icons.gpu
                }
                DeviceGroup {
                    visible: names.length > 1
                    kind: "gpu"
                    text: qsTr("Graphics")
                    iconName: root.icons.gpu
                    names: root.gpu.cardNames
                    labels: root.gpu.cardLabels
                    loads: root.devices.gpuUsages
                }
                NavItem {
                    visible: root.battery.packNames.length === 1
                    page: "battery:" + (root.battery.packNames[0] ?? "")
                    text: qsTr("Battery")
                    value: Format.percent(root.battery.packPercents[0] ?? NaN)
                    icon.name: root.icons.battery
                }
                DeviceGroup {
                    visible: names.length > 1
                    kind: "battery"
                    text: qsTr("Battery")
                    iconName: root.icons.battery
                    names: root.battery.packNames
                    labels: root.battery.packLabels
                    percents: root.battery.packPercents
                }
                NavItem {
                    visible: root.sensors.available
                    page: "sensors"
                    text: qsTr("Sensors")
                    icon.name: root.icons.sensors
                }
                NavHeading {
                    text: qsTr("System")
                }
                NavItem {
                    page: "apps"
                    text: qsTr("Apps")
                    icon.name: root.icons.apps
                }
                NavItem {
                    page: "energy"
                    text: qsTr("Energy Saver")
                    icon.name: root.icons.energy
                }
                NavItem {
                    page: "startup"
                    text: qsTr("Startup")
                    icon.name: root.icons.startup
                }
                NavItem {
                    page: "services"
                    text: qsTr("Services")
                    icon.name: root.icons.services
                }
                Item {
                    Layout.fillHeight: true
                }
                NavItem {
                    page: "settings"
                    text: qsTr("Settings")
                    icon.name: root.icons.settings
                }
                NavItem {
                    page: "about"
                    text: qsTr("About")
                    icon.name: root.icons.about
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

    // A drive, adapter or battery taken out while its page is open: the page
    // goes, rather than staying as dashes for something no longer there. A
    // device missing from one read only (a failed read) keeps its page:
    // leaving waits past the next read of the list, a slow tick away.
    function deviceListed() {
        const colon = currentPage.indexOf(":");
        const kind = currentPage.slice(0, colon);
        const device = currentPage.slice(colon + 1);
        const names = kind === "disk" ? devices.diskNames : kind === "network" ? devices.netNames : kind === "battery" ? battery.packNames : undefined;
        return colon < 0 || names === undefined || names.includes(device);
    }

    // The page left open last time, when it is a device's: it opens once
    // the first list of its kind has come in, if the device is still there.
    property string pendingPage: ""

    function restorePending(kind, names) {
        const prefix = kind + ":";
        if (pendingPage.startsWith(prefix)) {
            const page = pendingPage;
            pendingPage = "";
            if (names.includes(page.slice(prefix.length))) {
                showPage(page);
            }
        }
    }

    function checkDevice() {
        if (deviceListed()) {
            gone.stop();
        } else if (!gone.running) {
            gone.restart();
        }
    }

    Timer {
        id: gone
        // Two slow ticks (every fifth): disks and interfaces are listed again
        // on each.
        interval: root.backend.refreshInterval * 10 + 1000
        onTriggered: {
            if (root.deviceListed()) {
                return;
            }
            // One pack of two pulled: the one left is the total's page.
            const toTotal = root.currentPage.startsWith("battery:") && root.battery.packNames.includes("total");
            root.showPage(toTotal ? "battery:total" : "overview");
        }
    }

    Connections {
        target: root.devices
        function onDiskNamesChanged() {
            root.restorePending("disk", root.devices.diskNames);
            root.checkDevice();
        }
        function onNetNamesChanged() {
            root.restorePending("network", root.devices.netNames);
            root.checkDevice();
        }
    }

    Connections {
        target: root.gpu
        function onCardNamesChanged() {
            root.restorePending("gpu", root.gpu.cardNames);
        }
    }

    Connections {
        target: root.sensors
        function onAvailableChanged() {
            if (root.pendingPage === "sensors" && root.sensors.available) {
                root.showPage("sensors");
            }
        }
    }

    Connections {
        target: root.battery
        function onPackNamesChanged() {
            root.restorePending("battery", root.battery.packNames);
            root.checkDevice();
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
            icons: root.icons
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
            details: root.details
            hasGpu: root.gpu.cardNames.length > 0
        }
    }
    Component {
        id: cpuPage
        CpuPage {
            cpu: root.cpu
            backend: root.backend
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
            energy: root.energy
            onReleaseIdleMemory: root.releaseIdleMemory()
        }
    }
    Component {
        id: aboutPage
        AboutPage {
            backend: root.backend
        }
    }

    // Script objects nothing holds, then the window's caches (glyphs,
    // textures), which go as it draws its next frame; what they leave free
    // goes back to the system a moment after.
    function releaseIdleMemory() {
        gc();
        releaseResources();
        update();
        trim.restart();
    }
    Timer {
        id: trim
        interval: 500
        onTriggered: root.backend.releaseIdleMemory()
    }

    // Pages are saved as they change, from when the last one is back.
    property bool started: false
    onCurrentPageChanged: {
        if (started) {
            backend.savePage(currentPage);
        }
    }

    // A device's page waits for its list, which comes with the first tick.
    // A list that comes back without it, or not at all, leaves Overview.
    Timer {
        id: forgetPending
        interval: root.backend.refreshInterval * 2 + 1000
        onTriggered: root.pendingPage = ""
    }

    Component.onCompleted: {
        if (backend.windowMaximized) {
            showMaximized();
        }
        const last = backend.lastPage;
        const colon = last.indexOf(":");
        const lists = {
            "disk": devices.diskNames,
            "network": devices.netNames,
            "gpu": gpu.cardNames,
            "battery": battery.packNames
        };
        const kind = colon < 0 ? last : last.slice(0, colon);
        const names = colon >= 0 && Object.prototype.hasOwnProperty.call(lists, kind) ? lists[kind] : undefined;
        if (names !== undefined && names.includes(last.slice(colon + 1))) {
            showPage(last);
        } else if (last === "sensors" && !sensors.available) {
            // Like a device's page: Sensors is listed once the first read
            // finds one, and on a machine with none it never is.
            showPage("overview");
            pendingPage = last;
            forgetPending.start();
        } else {
            showPage(colon < 0 && isPage(last) ? last : "overview");
            if (names !== undefined && names.length === 0) {
                pendingPage = last;
                forgetPending.start();
            }
        }
        started = true;
    }

    // Size and maximized state are saved once a change settles: maximizing
    // can resize the window before it says it's maximized. Minimized or
    // full screen, the last of them stands.
    Timer {
        id: settle
        interval: 500
        onTriggered: {
            if (root.visibility === Window.Windowed || root.visibility === Window.Maximized) {
                root.backend.saveWindowSize(root.width, root.height, root.visibility === Window.Maximized);
            }
        }
    }
    onWidthChanged: settle.restart()
    onHeightChanged: settle.restart()
    onVisibilityChanged: settle.restart()

    function flushState() {
        if (settle.running) {
            settle.stop();
            settle.triggered();
        }
    }
    Connections {
        target: Qt.application
        function onAboutToQuit() {
            root.flushState();
        }
    }

    Shortcut {
        sequences: [StandardKey.Quit]
        onActivated: root.close()
    }

    onClosing: flushState()
}
