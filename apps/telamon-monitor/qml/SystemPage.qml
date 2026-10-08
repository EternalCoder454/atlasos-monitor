pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Telamon.Ui

// What this computer is: its system and desktop, its hardware as the
// firmware names it, and how well the firmware protects it (fwupd's Host
// Security ID). Copy Details puts it all on the clipboard, for a bug report.
TelamonPage {
    id: page

    required property var system
    required property var platform
    // The graphics cards, as the GPU page names them.
    required property var cards

    readonly property var s: page.system

    // The kernel's release, with its architecture unless it already says.
    readonly property string kernel: s.kernel === "" ? "" : s.arch === "" || s.kernel.includes(s.arch) ? s.kernel : qsTr("%1 (%2)").arg(s.kernel).arg(s.arch)

    // The firmware's version and date, "11.02 (5/5/2025)": its maker, which
    // is long, goes under the title instead (see `current`).
    readonly property string firmwareRelease: {
        const date = s.firmwareDate === "" ? "" : Format.date(s.firmwareDate);
        return s.firmwareVersion === "" ? date : date === "" ? s.firmwareVersion : qsTr("%1 (%2)").arg(s.firmwareVersion).arg(date);
    }

    // Every row, so the page and Copy Details say the same: a section, a
    // title, a value, and a subtitle for what goes under the title. Rows
    // with nothing to say are left out.
    readonly property var current: {
        const r = [];
        const add = (section, title, value, subtitle = "") => {
            if (value !== undefined && (value !== "" || subtitle !== "")) {
                r.push({
                    section: section,
                    title: title,
                    value: value,
                    subtitle: subtitle
                });
            }
        };
        add("software", qsTr("Operating System"), s.osName);
        add("software", qsTr("KDE Plasma"), s.plasma);
        add("software", qsTr("KDE Frameworks"), platform.frameworksVersion);
        add("software", qsTr("Qt"), platform.qtVersion);
        add("software", qsTr("Kernel"), page.kernel);
        add("software", qsTr("Graphics Platform"), platform.windowSystem);
        add("software", qsTr("Computer Name"), s.hostname);
        add("hardware", qsTr("Processor"), s.cpu === "" ? "" : s.cpuThreads > 0 ? qsTr("%1 × %2").arg(s.cpuThreads).arg(s.cpu) : s.cpu);
        add("hardware", qsTr("Memory"), s.memory > 0 ? qsTr("%1 usable").arg(Format.bytes(s.memory)) : "");
        for (let i = 0; i < page.cards.length; ++i) {
            add("hardware", qsTr("Graphics"), page.cards[i]);
        }
        add("hardware", qsTr("Manufacturer"), s.vendor);
        add("hardware", qsTr("Product"), s.product);
        add("hardware", qsTr("Type"), page.chassis(s.chassis));
        add("hardware", qsTr("Motherboard"), s.board);
        add("hardware", qsTr("Firmware"), page.firmwareRelease, s.firmwareVendor);
        return r;
    }

    // The rows the page shows: `current` once a change has settled. The
    // details arrive a property at a time, and each would make every row
    // again. Set, not bound, from when the page is made.
    property var rows: []
    onCurrentChanged: Qt.callLater(page.settle)

    function settle() {
        page.rows = page.current;
    }

    // The firmware's own word for the case, in this app's language.
    function chassis(word) {
        switch (word) {
        case "Desktop":
            return qsTr("Desktop");
        case "Laptop":
            return qsTr("Laptop");
        case "Convertible":
            return qsTr("Convertible");
        case "Tablet":
            return qsTr("Tablet");
        case "Mini PC":
            return qsTr("Mini PC");
        case "All-in-One":
            return qsTr("All-in-One");
        case "Server":
            return qsTr("Server");
        case "Handheld":
            return qsTr("Handheld");
        case "Docking Station":
            return qsTr("Docking Station");
        case "Detachable":
            return qsTr("Detachable");
        case "IoT Gateway":
            return qsTr("IoT Gateway");
        case "Embedded PC":
            return qsTr("Embedded PC");
        case "Stick PC":
            return qsTr("Stick PC");
        }
        return word;
    }

    // fwupd's levels, from HSI:0 (none of its checks pass) up; what each
    // level asks is in fwupd's documentation.
    function level(n) {
        switch (n) {
        case 0:
            return qsTr("Not protected");
        case 1:
            return qsTr("Basic protection");
        case 2:
            return qsTr("Good protection");
        case 3:
            return qsTr("Strong protection");
        case 4:
        case 5:
            return qsTr("Strongest protection");
        }
        return qsTr("Unknown");
    }

    readonly property string securityText: {
        if (!s.securityLoaded) {
            return "";
        }
        if (!s.securityAvailable || s.securityLevel < 0) {
            return qsTr("Not available");
        }
        return qsTr("%1 (level %2)").arg(page.level(s.securityLevel)).arg(s.securityLevel);
    }

    // The page as text: a heading per section, a line per row.
    function details() {
        const lines = [];
        const section = (key, heading) => {
            const own = page.rows.filter(row => row.section === key);
            if (own.length > 0) {
                lines.push(heading);
                own.forEach(row => lines.push(row.title + ": " + [row.subtitle, row.value].filter(t => t !== "").join(" ")));
                lines.push("");
            }
        };
        section("software", qsTr("Software"));
        section("hardware", qsTr("Hardware"));
        if (page.securityText !== "") {
            lines.push(qsTr("Firmware Security"));
            lines.push(qsTr("Level") + ": " + page.securityText + (s.securityRuntimeIssue ? " · " + qsTr("problem found while running") : ""));
            s.securityFailing.forEach(t => lines.push(qsTr("Failed") + ": " + t));
        }
        return lines.join("\n").trim() + "\n";
    }

    title: qsTr("System Info")

    RowLayout {
        Layout.fillWidth: true
        spacing: Kirigami.Units.largeSpacing

        Kirigami.Icon {
            Layout.preferredWidth: Kirigami.Units.iconSizes.huge
            Layout.preferredHeight: Kirigami.Units.iconSizes.huge
            source: page.s.osLogo || "computer"
            fallback: "computer"
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 0

            TelamonLabel {
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                textStyle: TelamonLabel.Heading
                text: page.s.product || page.s.vendor || page.s.hostname || qsTr("This Computer")
            }
            QQC2.Label {
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                opacity: 0.7
                textFormat: Text.PlainText
                text: page.s.loaded ? page.s.osName : qsTr("Reading this computer's details…")
            }
        }
        SecondaryButton {
            // Its width as Copy Details, so the row holds still while it
            // says it is done.
            property real fullWidth: 0
            Component.onCompleted: fullWidth = implicitWidth
            Layout.minimumWidth: fullWidth
            // Says it is done for a moment, in place.
            text: copied.running ? qsTr("Copied") : qsTr("Copy Details")
            icon.name: copied.running ? "checkmark" : "edit-copy"
            // Once fwupd has answered too, so the copy is complete.
            enabled: page.s.loaded && page.s.securityLoaded
            onClicked: {
                page.platform.copy(page.details());
                copied.restart();
            }

            Timer {
                id: copied
                interval: 2000
            }
        }
        SecondaryButton {
            text: qsTr("Refresh")
            icon.name: "view-refresh"
            enabled: !page.s.loading
            onClicked: page.s.refresh()
        }
    }

    Repeater {
        model: [
            {
                key: "software",
                title: qsTr("Software")
            },
            {
                key: "hardware",
                title: qsTr("Hardware")
            }
        ]

        Section {
            id: section
            required property var modelData
            readonly property var own: page.rows.filter(row => row.section === section.modelData.key)

            visible: own.length > 0
            title: modelData.title

            Repeater {
                model: section.own

                SectionRow {
                    id: row
                    required property var modelData
                    // A value beside its title is cut off past 55% of the row.
                    // One that would be goes under the title instead, where
                    // it wraps, so none of it is lost.
                    readonly property bool stacked: row.width > 0 && probe.implicitWidth > Math.round(row.width * 0.55)

                    title: modelData.title
                    subtitle: row.stacked ? [modelData.subtitle, modelData.value].filter(t => t !== "").join("\n") : modelData.subtitle
                    value: row.stacked ? "" : modelData.value

                    // The value's width as the row would draw it.
                    QQC2.Label {
                        id: probe
                        visible: false
                        text: row.modelData.value
                        textFormat: Text.PlainText
                    }
                }
            }
        }
    }

    Section {
        id: security
        property bool expanded: false

        visible: page.s.loaded
        title: qsTr("Firmware Security")
        footer: !page.s.securityLoaded ? "" : page.s.securityAvailable ? qsTr("Checked by fwupd %1 against its Host Security ID levels: each level adds protections the firmware and processor must turn on.").arg(page.s.fwupdVersion) : qsTr("fwupd checks how well the firmware protects this computer. It isn't installed, or it didn't answer.")

        SectionRow {
            title: qsTr("Level")
            value: page.s.securityLoaded ? page.securityText : qsTr("Checking…")
        }
        SectionRow {
            visible: page.s.securityRuntimeIssue
            title: qsTr("Problem Found While Running")
            subtitle: qsTr("Something in the running system weakens it, such as unencrypted swap or a tainted kernel.")

            Kirigami.Icon {
                Layout.preferredWidth: Kirigami.Units.iconSizes.small
                Layout.preferredHeight: Kirigami.Units.iconSizes.small
                source: "dialog-warning"
            }
        }
        SectionRow {
            visible: page.s.securityFailing.length > 0
            title: qsTr("Checks Not Passed")
            value: String(page.s.securityFailing.length)
            chevron: true
            disclosure: true
            expanded: security.expanded
            onClicked: security.expanded = !security.expanded
        }
        Repeater {
            model: security.expanded ? page.s.securityFailing : []

            SectionRow {
                required property string modelData
                title: modelData
            }
        }
    }

    Component.onCompleted: {
        page.rows = page.current;
        if (!page.s.loaded) {
            page.s.refresh();
        }
    }
}
