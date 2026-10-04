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
    required property var details
    // Whether the machine has a graphics card to show a GPU column for.
    required property bool hasGpu

    readonly property var actions: ({
            end: 0,
            kill: 1,
            stop: 2,
            resume: 3
        })
    function percent(v) {
        return Number(v).toLocaleString(Format.locale, "f", 1) + "%";
    }
    // Translated once: qsTr looks its file up in Qt's resources on every
    // call, and the cells below run on every row each second.
    readonly property string countFormat: qsTr("%1 (%2)")
    readonly property var powerNames: [qsTr("Very Low"), qsTr("Low"), qsTr("Moderate"), qsTr("High")]

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

    // The columns that can be hidden, for View's Columns and the header's
    // menu. Name always shows; GPU only with a card to show.
    readonly property var columnChoices: [
        {
            role: "pid",
            text: qsTr("PID")
        },
        {
            role: "cpu",
            text: qsTr("CPU")
        },
        {
            role: "memory",
            text: qsTr("Memory")
        },
        {
            role: "diskRead",
            text: qsTr("Disk Read")
        },
        {
            role: "diskWrite",
            text: qsTr("Disk Write")
        }
    ].concat(page.hasGpu ? [
            {
                role: "gpu",
                text: qsTr("GPU")
            }
        ] : []).concat([
        {
            role: "power",
            text: qsTr("Power")
        },
        {
            role: "netIn",
            text: qsTr("Net In")
        },
        {
            role: "netOut",
            text: qsTr("Net Out")
        }
    ])

    // A table sorted by a hidden column (hidden now, or saved hidden) sorts
    // by CPU instead, or by name when that is hidden too.
    function sortShown() {
        if (!page.apps.hiddenColumns.includes(table.sortRole)) {
            return;
        }
        const cpu = !page.apps.hiddenColumns.includes("cpu");
        table.sortOrder = cpu ? Qt.DescendingOrder : Qt.AscendingOrder;
        table.sortRole = cpu ? "cpu" : "name";
    }

    // Sorts once a header click has set both its column and order.
    function resort() {
        page.apps.sortBy(table.sortRole, table.sortOrder === Qt.DescendingOrder);
    }

    Shortcut {
        enabled: page.visible
        sequences: [StandardKey.Find]
        onActivated: search.forceActiveFocus()
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.topMargin: Kirigami.Units.gridUnit * 1.5
        anchors.bottomMargin: Kirigami.Units.gridUnit * 1.5
        anchors.leftMargin: Kirigami.Units.gridUnit * 1.5
        anchors.rightMargin: Kirigami.Units.gridUnit * 1.5
        spacing: Kirigami.Units.gridUnit

        // Typing over the table starts a search, as in a file manager:
        // the table passes on keys it has no use for, and they land here.
        // Only text: Escape, Backspace and Delete carry control characters.
        Keys.onPressed: event => {
            if (search.activeFocus || !/[^\s\x00-\x1f\x7f]/.test(event.text) || (event.modifiers & (Qt.ControlModifier | Qt.AltModifier | Qt.MetaModifier))) {
                return;
            }
            search.forceActiveFocus();
            search.insert(search.cursorPosition, event.text);
            event.accepted = true;
        }

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
                    checked: page.apps.kernelThreads
                    onTriggered: page.apps.showKernelThreads(checked)
                }
                QQC2.MenuSeparator {}
                QQC2.Menu {
                    id: viewColumns
                    title: qsTr("Columns")

                    Instantiator {
                        model: page.columnChoices
                        delegate: QQC2.MenuItem {
                            required property var modelData
                            text: modelData.text
                            checkable: true
                            checked: !page.apps.hiddenColumns.includes(modelData.role)
                            onTriggered: page.apps.setColumnShown(modelData.role, checked)
                        }
                        onObjectAdded: (index, object) => viewColumns.insertItem(index, object)
                        onObjectRemoved: (index, object) => viewColumns.removeItem(object)
                    }
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
                    // A row on its way out can lose its name for a moment.
                    text: (v, row) => row.count > 1 ? page.countFormat.arg(v).arg(row.count) : (v ?? "")
                },
                {
                    title: qsTr("PID"),
                    role: "pid",
                    width: 3.5,
                    align: Qt.AlignRight,
                    text: v => v > 0 ? String(v) : ""
                },
                {
                    title: qsTr("CPU"),
                    role: "cpu",
                    width: 4,
                    align: Qt.AlignRight,
                    heat: 100,
                    text: v => page.percent(v)
                },
                {
                    title: qsTr("Memory"),
                    role: "memory",
                    width: 5,
                    align: Qt.AlignRight,
                    // Tinted as the Go version's: faint at a few hundred
                    // megabytes, full at 4 GiB.
                    heat: 4294967296,
                    text: v => Format.size(v)
                },
                {
                    title: qsTr("Disk Read"),
                    role: "diskRead",
                    width: 5.5,
                    align: Qt.AlignRight,
                    text: v => Format.rate(v)
                },
                {
                    title: qsTr("Disk Write"),
                    role: "diskWrite",
                    width: 5.5,
                    align: Qt.AlignRight,
                    text: v => Format.rate(v)
                },
            ].concat(page.hasGpu ? [
                {
                    title: qsTr("GPU"),
                    role: "gpu",
                    width: 4,
                    align: Qt.AlignRight,
                    heat: 100,
                    text: v => isNaN(v) ? Format.dash : page.percent(v)
                }
            ] : []).concat([
                {
                    title: qsTr("Power"),
                    role: "power",
                    width: 5,
                    text: v => page.powerNames[v] ?? ""
                },
                {
                    // Estimates: the machine's traffic shared out by each
                    // program's open sockets.
                    title: qsTr("Net ≈ In"),
                    role: "netIn",
                    width: 5.5,
                    align: Qt.AlignRight,
                    text: v => Format.rate(v)
                },
                {
                    title: qsTr("Net ≈ Out"),
                    role: "netOut",
                    width: 5.5,
                    align: Qt.AlignRight,
                    text: v => Format.rate(v)
                }
            ]).filter(c => c.role === "name" || !page.apps.hiddenColumns.includes(c.role))

            onSortRoleChanged: Qt.callLater(page.resort)
            onSortOrderChanged: Qt.callLater(page.resort)
            onToggleRequested: row => page.apps.toggle(row)
            // Double-click or Enter, as in every other file and process list.
            onActivated: row => {
                if (page.apps.pin(row)) {
                    page.apps.showDetails();
                }
            }
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
            onHeaderMenuRequested: (x, y) => columnsMenu.popup(table, x, y)

            // Rows hold still under the pointer, and while a menu or a
            // question is up about one of them.
            readonly property bool held: pointerInside || rowMenu.opened || endDialog.opened || killDialog.opened || detailsDialog.visible
            onHeldChanged: page.apps.setHeld(held)
        }
    }

    // The header's right-click menu: the columns, a check by each shown.
    ContextMenu {
        id: columnsMenu

        Instantiator {
            model: page.columnChoices
            delegate: ContextMenuItem {
                required property var modelData
                readonly property bool shown: !page.apps.hiddenColumns.includes(modelData.role)
                text: modelData.text
                // ContextMenuItem draws no check box of its own.
                icon.name: shown ? "checkmark" : ""
                Accessible.checkable: true
                Accessible.checked: shown
                onTriggered: page.apps.setColumnShown(modelData.role, !shown)
            }
            onObjectAdded: (index, object) => columnsMenu.insertItem(index, object)
            onObjectRemoved: (index, object) => columnsMenu.removeItem(object)
        }
    }

    Connections {
        target: page.apps
        function onHiddenColumnsChanged() {
            page.sortShown();
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
        function onModelReset() {
            table.currentIndex = -1;
        }
        // No file manager took the program: its folder opens instead.
        function onLocated(name, folder) {
            if (folder.length > 0) {
                Qt.openUrlExternally(folder);
            } else {
                notice.show(qsTr("Atlas Monitor can't find where %1's program is.").arg(name));
            }
        }
    }

    Connections {
        target: page.details
        function onShown() {
            detailsDialog.open();
        }
    }

    ContextMenu {
        id: rowMenu
        ContextMenuItem {
            text: qsTr("Details")
            icon.name: "documentinfo"
            onTriggered: page.apps.showDetails()
        }
        ContextMenuItem {
            text: qsTr("Open File Location")
            icon.name: "folder-open"
            onTriggered: page.apps.openLocation()
        }
        ContextMenuSeparator {}
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

    DetailsDialog {
        id: detailsDialog
        details: page.details
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
        // A hold left over from a page closed under the pointer.
        page.apps.setHeld(false);
        page.sortShown();
        page.resort();
        page.apps.setSearch(search.query);
    }
    Component.onDestruction: page.apps.setHeld(false)
}
