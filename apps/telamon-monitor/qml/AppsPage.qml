pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Telamon.Ui

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
            notice.show(qsTr("%1 belongs to another user or to the system, so Telamon Monitor isn't allowed to do that.").arg(name));
        } else if (result === "gone") {
            notice.show(qsTr("%1 had already closed.").arg(name));
        } else if (result === "failed") {
            notice.show(qsTr("That didn't work for %1.").arg(name));
        }
    }

    // The menus and dialogs are made the first time they open, not with the
    // page: together they are 5 MiB.
    function made(loader) {
        loader.active = true;
        return loader.item;
    }
    function isOpen(loader) {
        return loader.item !== null && loader.item.opened;
    }
    function isVisible(loader) {
        return loader.item !== null && loader.item.visible;
    }

    // End Task asks first only for an application of several processes:
    // one process asked to close can still save its work.
    function endTask() {
        if (page.apps.pinnedCount > 1) {
            page.made(endDialogLoader).open();
        } else {
            page.act(page.actions.end);
        }
    }

    // The text of the table's cells and headers, measured, so a column is as
    // wide as its widest realistic figure and its header and never shows
    // "1…" for 100.0% (DataTable cuts what doesn't fit). Figures are drawn
    // with tabular digits, the headers in DemiBold when sorted.
    QQC2.Label {
        id: figureFont
        textFormat: Text.PlainText
        visible: false
        font.features: {
            "tnum": 1
        }
    }
    QQC2.Label {
        id: headFont
        textFormat: Text.PlainText
        visible: false
        font.weight: Font.DemiBold
    }
    FontMetrics {
        id: figureMetrics
        font: figureFont.font
    }
    FontMetrics {
        id: headMetrics
        font: headFont.font
    }

    // A column's width in grid units: the widest of `samples` or the title,
    // plus the table's cell padding and, for the title, the sort arrow it
    // keeps room for.
    function columnWidth(title, samples) {
        // Read so a change of font measures again (advanceWidth() is a call,
        // which a binding can't see).
        void (figureMetrics.font.pixelSize + figureMetrics.font.pointSize + headMetrics.font.pixelSize + headMetrics.font.pointSize);
        const pad = 2 * TelamonStyle.spacingLarge;
        const cell = Math.max(...samples.map(t => figureMetrics.advanceWidth(t))) + pad;
        const head = headMetrics.advanceWidth(title) + pad + Kirigami.Units.iconSizes.small;
        return Math.ceil(Math.max(cell, head) + 4) / Kirigami.Units.gridUnit;
    }

    // Every column but Name, in the order the table shows them. `menu` is
    // the name in the Columns menus, `tip` what the short header stands for.
    // Estimates: the machine's traffic shared out by each program's open
    // sockets. `narrow` ranks the columns that go first when the window is
    // too narrow for all of them (see `autoHidden`); CPU never goes.
    readonly property var figureSpecs: [
        {
            role: "pid",
            title: qsTr("PID"),
            menu: qsTr("PID"),
            tip: qsTr("Process ID"),
            narrow: 6,
            samples: ["4194304"],
            text: v => v > 0 ? String(v) : ""
        },
        {
            role: "cpu",
            title: qsTr("CPU"),
            menu: qsTr("CPU"),
            tip: qsTr("How much of the processor it uses"),
            narrow: 100,
            samples: [page.percent(3200)],
            heat: 100,
            text: v => page.percent(v)
        },
        {
            role: "memory",
            title: qsTr("Memory"),
            menu: qsTr("Memory"),
            tip: qsTr("Memory it holds"),
            narrow: 7,
            samples: [Format.size(1023.9 * 1048576), Format.size(120.5 * 1073741824)],
            // Tinted as the Go version's: faint at a few hundred
            // megabytes, full at 4 GiB.
            heat: 4294967296,
            text: v => Format.size(v)
        },
        {
            role: "diskRead",
            title: qsTr("Disk Read"),
            menu: qsTr("Disk Read"),
            tip: qsTr("How fast it reads from disks"),
            narrow: 4,
            samples: [Format.rate(999.9 * 1048576), Format.rate(9.99 * 1073741824)],
            text: v => Format.rate(v)
        },
        {
            role: "diskWrite",
            title: qsTr("Disk Write"),
            menu: qsTr("Disk Write"),
            tip: qsTr("How fast it writes to disks"),
            narrow: 3,
            samples: [Format.rate(999.9 * 1048576), Format.rate(9.99 * 1073741824)],
            text: v => Format.rate(v)
        },
        {
            role: "gpu",
            title: qsTr("GPU"),
            menu: qsTr("GPU"),
            tip: qsTr("How much of the graphics card it uses"),
            narrow: 5,
            samples: [page.percent(100)],
            heat: 100,
            text: v => isNaN(v) ? Format.dash : page.percent(v)
        },
        {
            role: "power",
            title: qsTr("Power"),
            menu: qsTr("Power"),
            tip: qsTr("How much power it uses, roughly"),
            narrow: 2,
            samples: page.powerNames,
            align: Qt.AlignLeft,
            text: v => page.powerNames[v] ?? ""
        },
        {
            role: "netIn",
            title: qsTr("Net ↓"),
            menu: qsTr("Net In (Estimated)"),
            tip: qsTr("Network download speed, estimated"),
            narrow: 1,
            samples: [Format.rate(999.9 * 1048576)],
            text: v => Format.rate(v)
        },
        {
            role: "netOut",
            title: qsTr("Net ↑"),
            menu: qsTr("Net Out (Estimated)"),
            tip: qsTr("Network upload speed, estimated"),
            narrow: 0,
            samples: [Format.rate(999.9 * 1048576)],
            text: v => Format.rate(v)
        }
    ]
    // Each with its measured width, `width`, as DataTable takes it.
    readonly property var figureColumns: page.figureSpecs.map(c => Object.assign({
            width: page.columnWidth(c.title, c.samples),
            align: Qt.AlignRight
        }, c))

    // The columns that can be hidden, for View's Columns and the header's
    // menu. Name always shows; GPU only with a card to show.
    readonly property var columnChoices: page.figureColumns.filter(c => c.role !== "gpu" || page.hasGpu).map(c => ({
                role: c.role,
                text: c.menu
            }))

    // The columns the window is too narrow for, as "role,role": Name keeps
    // at least `nameMinimum`, so figures are never cut off. They go in the
    // order `narrow` gives, skipping any the user already hid and the one
    // the table is sorted by. A string, so the table's columns change only
    // when the set does, not at every pixel of a resize.
    readonly property real nameMinimum: Kirigami.Units.gridUnit * 13
    readonly property string autoHidden: {
        const hidden = [];
        let room = table.width - 2 * table.padding - page.nameMinimum;
        const shown = page.figureColumns.filter(c => (c.role !== "gpu" || page.hasGpu) && !page.apps.hiddenColumns.includes(c.role));
        const px = c => c.width * Kirigami.Units.gridUnit;
        room -= shown.reduce((sum, c) => sum + px(c), 0);
        const order = shown.slice().sort((a, b) => a.narrow - b.narrow);
        for (const c of order) {
            if (room >= 0 || c.narrow >= 100) {
                break;
            }
            if (c.role !== table.sortRole) {
                hidden.push(c.role);
                room += px(c);
            }
        }
        return hidden.join(",");
    }

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

            TelamonLabel {
                Layout.fillWidth: true
                text: qsTr("Apps")
                textStyle: TelamonLabel.Title
                elide: Text.ElideRight
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

                    // Last, once the Instantiator's items are in before it.
                    QQC2.MenuItem {
                        visible: page.autoHidden !== ""
                        enabled: false
                        // Empty when not needed: a menu is as wide as its widest item, shown or not.
                        text: page.autoHidden !== "" ? qsTr("Columns the window is too narrow for are left out") : ""
                    }
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

        InfoBanner {
            id: notice
            Layout.fillWidth: true
            type: "warning"
            closable: true
            // Shown by what it reports.
            shown: false

            function show(message) {
                notice.text = message;
                notice.shown = true;
                hide.restart();
            }

            Timer {
                id: hide
                interval: 8000
                onTriggered: notice.shown = false
            }
        }

        DataTable {
            id: table
            Layout.fillWidth: true
            Layout.fillHeight: true
            // The table's text comes from an invisible Label (DataTable's
            // `cellLabel`), and an invisible item gets no colours from the
            // Qt style's colour scheme: with a light Kvantum theme loaded
            // under a dark scheme its text came out black on the dark card.
            // Set here, every Label and icon in the table follows Telamon.Ui's
            // text colour, whatever the style.
            Kirigami.Theme.textColor: TelamonStyle.text
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
                }
            ].concat(page.figureColumns.filter(c => (c.role !== "gpu" || page.hasGpu) && !page.apps.hiddenColumns.includes(c.role) && !page.autoHidden.split(",").includes(c.role)))

            // Short headers say what they are on hover (the table's headers
            // take a title and nothing else): one invisible cell over each.
            Row {
                x: table.padding
                y: table.padding
                width: table.width - 2 * table.padding
                height: Math.round(Kirigami.Units.gridUnit * 1.8)

                Repeater {
                    model: table.columns

                    Item {
                        id: headTip
                        required property int index
                        required property var modelData
                        width: table.widths[index] ?? 0
                        height: parent.height

                        HoverHandler {
                            id: headHover
                        }
                        TelamonToolTip {
                            text: headTip.modelData.tip ?? ""
                            shown: headHover.hovered
                        }
                    }
                }
            }

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
                    page.made(rowMenuLoader).popup(table, x, y);
                }
            }
            onHeaderMenuRequested: (x, y) => page.made(columnsMenuLoader).popup(table, x, y)

            // Rows hold still under the pointer, and while a menu or a
            // question is up about one of them.
            readonly property bool held: pointerInside || page.isOpen(rowMenuLoader) || page.isOpen(endDialogLoader) || page.isOpen(killDialogLoader) || page.isVisible(detailsDialogLoader)
            onHeldChanged: page.apps.setHeld(held)
        }
    }

    // The header's right-click menu: the columns, a check by each shown.
    Loader {
        id: columnsMenuLoader
        active: false
        sourceComponent: ContextMenu {
            id: columnsMenu
            parent: page

            ContextMenuItem {
                visible: page.autoHidden !== ""
                enabled: false
                // Empty when not needed: a menu is as wide as its widest item, shown or not.
                text: page.autoHidden !== "" ? qsTr("Columns the window is too narrow for are left out") : ""
            }
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
                notice.show(qsTr("Telamon Monitor can't find where %1's program is.").arg(name));
            }
        }
    }

    Connections {
        target: page.details
        function onShown() {
            page.made(detailsDialogLoader).open();
        }
    }

    Loader {
        id: rowMenuLoader
        active: false
        sourceComponent: ContextMenu {
            id: rowMenu
            parent: page
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
                onTriggered: page.made(killDialogLoader).open()
            }
        }
    }

    Loader {
        id: detailsDialogLoader
        active: false
        sourceComponent: DetailsDialog {
            id: detailsDialog
            details: page.details
        }
    }

    Loader {
        id: endDialogLoader
        active: false
        sourceComponent: ConfirmDialog {
            id: endDialog
            title: qsTr("End %1?").arg(page.apps.pinnedName)
            text: qsTr("Each of its %1 processes is asked to close. Anything it hasn't saved may be lost.").arg(page.apps.pinnedCount)
            acceptText: qsTr("End Task")
            focusReject: true
            onAccepted: page.act(page.actions.end)
        }
    }

    Loader {
        id: killDialogLoader
        active: false
        sourceComponent: ConfirmDialog {
            id: killDialog
            title: qsTr("Kill %1?").arg(page.apps.pinnedName)
            text: qsTr("It stops at once, with no chance to save. Use this only for something that doesn't respond to End Task.")
            acceptText: qsTr("Kill")
            focusReject: true
            onAccepted: page.act(page.actions.kill)
        }
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
