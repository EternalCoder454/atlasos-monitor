pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

// The system's services, failed ones first, each with a status dot. Start,
// Stop, Restart, Enable and Disable go to systemd, which has polkit ask for
// a password; the model (src/services.rs) does that off the GUI thread and
// answers with `acted`.
Item {
    id: page

    required property var services
    required property var sampler

    readonly property var actions: ({
            start: 0,
            stop: 1,
            restart: 2,
            enable: 3,
            disable: 4
        })
    // The service a menu or the Stop question is about, by name: rows move
    // as the list refreshes, a name doesn't.
    property string target
    property int available: 0

    function startup(key) {
        switch (key) {
        case "enabled":
            return qsTr("On");
        case "disabled":
            return qsTr("Off");
        case "static":
            return qsTr("As Needed");
        case "enabled-runtime":
            return qsTr("On Until Restart");
        case "masked":
        case "masked-runtime":
            return qsTr("Blocked");
        case "indirect":
            return qsTr("Indirect");
        case "alias":
            return qsTr("Alias");
        case "generated":
            return qsTr("Generated");
        case "transient":
            return qsTr("Temporary");
        case "bad":
            return qsTr("Broken");
        case "linked":
        case "linked-runtime":
            return qsTr("Linked");
        case "":
            return Format.dash;
        }
        return key;
    }

    function status(key, job) {
        if (job === "start" || job === "restart")
            return qsTr("Starting");
        if (job === "stop")
            return qsTr("Stopping");
        switch (key) {
        case "running":
            return qsTr("Running");
        case "active":
            return qsTr("Active");
        case "starting":
            return qsTr("Starting");
        case "stopping":
            return qsTr("Stopping");
        case "failed":
            return qsTr("Failed");
        }
        return qsTr("Stopped");
    }

    function failure(result, name, detail) {
        switch (result) {
        case "stillRunning":
            return qsTr("%1 is still starting. The list shows how it ends.").arg(name);
        case "notEnableable":
            return qsTr("%1 has nothing to turn on at startup: other services start it when they need it.").arg(name);
        case "notAllowed":
            return qsTr("You weren't allowed to change %1.").arg(name);
        case "noSuchUnit":
            return qsTr("%1 is no longer installed.").arg(name);
        case "masked":
            return qsTr("%1 is blocked, so it can't be started.").arg(name);
        case "failed":
            return qsTr("%1 failed.").arg(name);
        case "dependency":
            return qsTr("%1 didn't start: a service it needs failed.").arg(name);
        case "timeout":
            return qsTr("%1 took too long and was stopped.").arg(name);
        case "canceled":
            return qsTr("Another change to %1 took the place of this one.").arg(name);
        case "refused":
            return qsTr("systemd refused to change %1: %2").arg(name).arg(detail);
        case "noAnswer":
            return qsTr("systemd didn't answer about %1.").arg(name);
        }
        return "";
    }

    function act(action) {
        if (!page.services.act(page.target, action)) {
            notice.show(qsTr("Wait for the change under way to finish."), Kirigami.MessageType.Information);
        }
    }

    function can(action) {
        return (page.available & (1 << action)) !== 0;
    }

    Shortcut {
        enabled: page.visible
        sequence: StandardKey.Find
        onActivated: search.forceActiveFocus()
    }

    Connections {
        target: page.services
        function onActed(name, action, result, detail) {
            // Enable and Disable change the unit files, which the list reads
            // only when told.
            page.sampler.servicesChanged();
            const text = page.failure(result, name, detail);
            if (text.length > 0) {
                notice.show(text, result === "stillRunning" ? Kirigami.MessageType.Information : Kirigami.MessageType.Warning);
            }
        }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.topMargin: Kirigami.Units.gridUnit * 1.5
        anchors.bottomMargin: Kirigami.Units.gridUnit * 1.5
        anchors.leftMargin: Kirigami.Units.gridUnit * 1.5
        anchors.rightMargin: Kirigami.Units.gridUnit * 1.5
        spacing: Kirigami.Units.gridUnit

        RowLayout {
            Layout.fillWidth: true
            spacing: Kirigami.Units.largeSpacing

            QQC2.Label {
                Layout.fillWidth: true
                text: qsTr("Services")
                font.pointSize: Kirigami.Theme.defaultFont.pointSize * 1.6
                font.bold: true
                elide: Text.ElideRight
                Accessible.role: Accessible.Heading
            }
            QQC2.Label {
                text: page.services.failedCount > 0 ? qsTr("Failed Only (%1)").arg(page.services.failedCount) : qsTr("Failed Only")
                opacity: failedOnly.enabled ? 1 : 0.5
            }
            AtlasSwitch {
                id: failedOnly
                // Nothing to show with none failed, unless it's on already.
                enabled: checked || page.services.failedCount > 0
                onToggled: page.services.setProblemsOnly(checked)
                Accessible.name: qsTr("Failed Only")
            }
            SearchField {
                id: search
                placeholderText: qsTr("Search Services")
                onQueryChanged: page.services.setSearch(query)
            }
        }

        Kirigami.InlineMessage {
            id: notice
            Layout.fillWidth: true
            showCloseButton: true

            function show(message, kind) {
                notice.type = kind;
                notice.text = message;
                notice.visible = true;
                hide.restart();
            }

            Timer {
                id: hide
                interval: 10000
                onTriggered: notice.visible = false
            }
        }

        Kirigami.InlineMessage {
            Layout.fillWidth: true
            type: Kirigami.MessageType.Information
            visible: page.services.busy
            text: qsTr("Waiting for systemd. If it asks for your password, the change goes ahead once you give it.")
        }

        Kirigami.InlineMessage {
            Layout.fillWidth: true
            type: Kirigami.MessageType.Warning
            visible: page.services.loaded && !page.services.available && page.services.count > 0
            text: qsTr("systemd didn't answer, so this list may be out of date.")
        }

        DataTable {
            id: table
            Layout.fillWidth: true
            Layout.fillHeight: true
            Accessible.name: qsTr("Services")
            model: page.services
            sortRole: "status"
            placeholderText: !page.services.loaded ? qsTr("Reading services…") : !page.services.available ? qsTr("systemd didn't answer, so services can't be listed.") : search.query.length > 0 ? qsTr("No Services Match") : failedOnly.checked ? qsTr("No Service Has Failed") : ""
            columns: [
                {
                    title: qsTr("Status"),
                    role: "status",
                    width: 7,
                    cell: statusCell
                },
                {
                    title: qsTr("Service"),
                    role: "name",
                    width: 14
                },
                {
                    title: qsTr("Description"),
                    role: "description",
                    fill: true
                },
                {
                    title: qsTr("Startup"),
                    role: "startup",
                    width: 7,
                    text: v => page.startup(v)
                }
            ]

            onSortRoleChanged: Qt.callLater(page.resort)
            onSortOrderChanged: Qt.callLater(page.resort)
            onContextMenuRequested: (row, x, y) => {
                page.target = page.services.nameAt(row);
                page.available = page.services.availableAt(row);
                if (page.target.length > 0) {
                    rowMenu.popup(table, x, y);
                }
            }
        }
    }

    function resort() {
        page.services.sortBy(table.sortRole, table.sortOrder === Qt.DescendingOrder);
    }

    // A row's selection doesn't survive its row going.
    Connections {
        target: page.services
        function onRowsAboutToBeRemoved(parent, first, last) {
            if (table.currentIndex >= first && table.currentIndex <= last) {
                table.currentIndex = -1;
            }
        }
        function onModelReset() {
            table.currentIndex = -1;
        }
    }

    Component {
        id: statusCell
        RowLayout {
            property var value
            property var row
            property var column

            spacing: Kirigami.Units.smallSpacing

            Rectangle {
                Layout.alignment: Qt.AlignVCenter
                implicitWidth: Kirigami.Units.gridUnit * 0.5
                implicitHeight: implicitWidth
                radius: width / 2
                color: {
                    switch (parent.value) {
                    case "failed":
                        return Kirigami.Theme.negativeTextColor;
                    case "running":
                        return Kirigami.Theme.positiveTextColor;
                    case "active":
                        return Qt.alpha(Kirigami.Theme.positiveTextColor, 0.45);
                    case "starting":
                    case "stopping":
                        return Kirigami.Theme.neutralTextColor;
                    }
                    return Qt.alpha(Kirigami.Theme.textColor, 0.25);
                }
            }
            QQC2.Label {
                Layout.fillWidth: true
                text: page.status(parent.value, parent.row ? parent.row.job : "")
                elide: Text.ElideRight
                color: parent.value === "failed" ? Kirigami.Theme.negativeTextColor : Kirigami.Theme.textColor
            }
        }
    }

    ContextMenu {
        id: rowMenu
        ContextMenuItem {
            text: qsTr("Start")
            icon.name: "media-playback-start"
            enabled: page.can(page.actions.start)
            onTriggered: page.act(page.actions.start)
        }
        ContextMenuItem {
            text: qsTr("Restart")
            icon.name: "view-refresh"
            enabled: page.can(page.actions.restart)
            onTriggered: page.act(page.actions.restart)
        }
        ContextMenuItem {
            text: qsTr("Stop")
            icon.name: "media-playback-stop"
            destructive: true
            enabled: page.can(page.actions.stop)
            onTriggered: stopDialog.open()
        }
        ContextMenuSeparator {}
        ContextMenuItem {
            text: qsTr("Start at Boot")
            icon.name: "checkmark"
            enabled: page.can(page.actions.enable)
            onTriggered: page.act(page.actions.enable)
        }
        ContextMenuItem {
            text: qsTr("Don't Start at Boot")
            icon.name: "action-unavailable"
            enabled: page.can(page.actions.disable)
            onTriggered: page.act(page.actions.disable)
        }
    }

    ConfirmDialog {
        id: stopDialog
        title: qsTr("Stop %1?").arg(page.target)
        text: qsTr("Anything that needs it may stop working until it is started again or the computer restarts.")
        acceptText: qsTr("Stop")
        focusReject: true
        onAccepted: page.act(page.actions.stop)
    }

    Component.onCompleted: {
        page.resort();
        page.services.setSearch(search.query);
        page.services.setProblemsOnly(false);
    }
}
