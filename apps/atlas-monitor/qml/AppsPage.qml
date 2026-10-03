pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

// Every running application, or every process, with what each costs. The
// model (src/processes.rs) sorts, searches and groups; this page lays it
// out, holds its order still under the pointer, and asks before anything
// that can lose someone's work.
Item {
    id: page

    required property var apps
    required property var sampler
    // Whether the machine has a graphics card to show a GPU column for.
    required property bool hasGpu

    readonly property var actions: ({
            end: 0,
            kill: 1,
            stop: 2,
            resume: 3
        })
    function percent(v) {
        return Number(v).toLocaleString(Qt.locale(), "f", 1) + "%";
    }

    // Does `action` to the pinned row's processes (the model's `pin`), and
    // says why if it couldn't.
    function act(action) {
        const name = page.apps.pinnedName;
        const result = page.apps.act(action);
        if (result === "denied") {
            notice.show(qsTr("%1 belongs to another user or to the system, so Atlas Monitor isn't allowed to do that.").arg(name));
        } else if (result === "gone") {
            notice.show(qsTr("%1 had already closed.").arg(name));
        } else if (result === "failed") {
            notice.show(qsTr("That didn't work for %1.").arg(name));
        }
    }

    // End Task asks first only for an application of several processes:
    // one process asked to close can still save its work.
    function endTask() {
        if (page.apps.pinnedCount > 1) {
            endDialog.open();
        } else {
            page.act(page.actions.end);
        }
    }

    // Sorts once a header click has set both its column and order.
    function resort() {
        page.apps.sortBy(table.sortRole, table.sortOrder === Qt.DescendingOrder);
    }

    Shortcut {
        enabled: page.visible
        sequence: StandardKey.Find
        onActivated: search.forceActiveFocus()
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
                text: qsTr("Apps")
                font.pointSize: Kirigami.Theme.defaultFont.pointSize * 1.6
                font.bold: true
                elide: Text.ElideRight
                Accessible.role: Accessible.Heading
            }
            SearchField {
                id: search
                placeholderText: qsTr("Search Apps")
                onQueryChanged: page.apps.setSearch(query)
            }
            MenuButton {
                text: qsTr("View")
                QQC2.MenuItem {
                    text: qsTr("Group by App")
                    checkable: true
                    checked: page.apps.grouped
                    onTriggered: page.apps.setGrouping(checked)
                }
                QQC2.MenuItem {
                    text: qsTr("Show Kernel Threads")
                    checkable: true
                    onTriggered: page.sampler.showKernelThreads(checked)
                }
            }
        }

        Kirigami.InlineMessage {
            id: notice
            Layout.fillWidth: true
            type: Kirigami.MessageType.Warning
            showCloseButton: true

            function show(message) {
                notice.text = message;
                notice.visible = true;
                hide.restart();
            }

            Timer {
                id: hide
                interval: 8000
                onTriggered: notice.visible = false
            }
        }

        DataTable {
            id: table
            Layout.fillWidth: true
            Layout.fillHeight: true
            Accessible.name: qsTr("Apps")
            model: page.apps
            sortRole: "cpu"
            depthRole: "depth"
            expandableRole: "expandable"
            expandedRole: "expanded"
            placeholderText: search.query.length > 0 ? qsTr("No Apps Match") : ""
            columns: [
                {
                    title: qsTr("Name"),
                    role: "name",
                    fill: true,
                    iconRole: "icon",
                    // An application's row says how many processes it is.
                    text: (v, row) => row.count > 1 ? qsTr("%1 (%2)").arg(v).arg(row.count) : v
                },
                {
                    title: qsTr("PID"),
                    role: "pid",
                    width: 4,
                    align: Qt.AlignRight,
                    text: v => v > 0 ? String(v) : ""
                },
                {
                    title: qsTr("CPU"),
                    role: "cpu",
                    width: 5,
                    align: Qt.AlignRight,
                    heat: 100,
                    text: v => page.percent(v)
                },
                {
                    title: qsTr("Memory"),
                    role: "memory",
                    width: 6,
                    align: Qt.AlignRight,
                    text: v => Format.size(v)
                },
            ].concat(page.hasGpu ? [
                {
                    title: qsTr("GPU"),
                    role: "gpu",
                    width: 5,
                    align: Qt.AlignRight,
                    heat: 100,
                    text: v => isNaN(v) ? Format.dash : page.percent(v)
                }
            ] : []).concat([
                {
                    title: qsTr("Power"),
                    role: "power",
                    width: 5,
                    text: v => [qsTr("Very Low"), qsTr("Low"), qsTr("Moderate"), qsTr("High")][v] ?? ""
                },
                {
                    // Estimates: the machine's traffic shared out by each
                    // program's open sockets.
                    title: qsTr("Net ≈ In"),
                    role: "netIn",
                    width: 6,
                    align: Qt.AlignRight,
                    text: v => Format.rate(v)
                },
                {
                    title: qsTr("Net ≈ Out"),
                    role: "netOut",
                    width: 6,
                    align: Qt.AlignRight,
                    text: v => Format.rate(v)
                }
            ])

            onSortRoleChanged: Qt.callLater(page.resort)
            onSortOrderChanged: Qt.callLater(page.resort)
            onToggleRequested: row => page.apps.toggle(row)
            onDeleteRequested: row => {
                if (page.apps.pin(row)) {
                    page.endTask();
                }
            }
            // The row is pinned as the menu opens: whatever the list does
            // while the menu or a question is up, the answer goes to it.
            onContextMenuRequested: (row, x, y) => {
                if (page.apps.pin(row)) {
                    rowMenu.popup(table, x, y);
                }
            }

            // Rows hold still under the pointer, and while a menu or a
            // question is up about one of them.
            readonly property bool held: pointerInside || rowMenu.opened || endDialog.opened || killDialog.opened
            onHeldChanged: page.apps.setHeld(held)
        }
    }

    // The selected row going (its process exited) leaves nothing selected,
    // rather than the row that slides into its place: Delete would end that.
    Connections {
        target: page.apps
        function onRowsAboutToBeRemoved(parent, first, last) {
            if (table.currentIndex >= first && table.currentIndex <= last) {
                table.currentIndex = -1;
            }
        }
    }

    ContextMenu {
        id: rowMenu
        ContextMenuItem {
            text: qsTr("Stop")
            icon.name: "media-playback-pause"
            onTriggered: page.act(page.actions.stop)
        }
        ContextMenuItem {
            text: qsTr("Continue")
            icon.name: "media-playback-start"
            onTriggered: page.act(page.actions.resume)
        }
        ContextMenuSeparator {}
        ContextMenuItem {
            text: qsTr("End Task")
            icon.name: "process-stop"
            shortcutText: qsTr("Del")
            destructive: true
            onTriggered: page.endTask()
        }
        ContextMenuItem {
            text: qsTr("Kill")
            icon.name: "edit-bomb"
            destructive: true
            onTriggered: killDialog.open()
        }
    }

    ConfirmDialog {
        id: endDialog
        title: qsTr("End %1?").arg(page.apps.pinnedName)
        text: qsTr("Each of its %1 processes is asked to close. Anything it hasn't saved may be lost.").arg(page.apps.pinnedCount)
        acceptText: qsTr("End Task")
        focusReject: true
        onAccepted: page.act(page.actions.end)
    }

    ConfirmDialog {
        id: killDialog
        title: qsTr("Kill %1?").arg(page.apps.pinnedName)
        text: qsTr("It stops at once, with no chance to save. Use this only for something that doesn't respond to End Task.")
        acceptText: qsTr("Kill")
        focusReject: true
        onAccepted: page.act(page.actions.kill)
    }

    Component.onCompleted: {
        // Kernel threads are left out each time the page opens; the menu
        // starts unticked to match.
        page.sampler.showKernelThreads(false);
        // A hold left over from a page closed under the pointer.
        page.apps.setHeld(false);
        page.resort();
        page.apps.setSearch(search.query);
    }
    Component.onDestruction: page.apps.setHeld(false)
}
