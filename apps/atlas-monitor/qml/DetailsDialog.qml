pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

// What a process is, before somebody decides to end it: a row called
// "python3" doesn't say what started it or what it runs. For an application
// of several processes, the application and its busiest members, each of
// which opens here in turn. The figures are read once, as it opens
// (src/details.rs).
QQC2.Popup {
    id: dialog

    required property var details

    // "sleeping" as "Sleeping".
    function capitalise(s) {
        return s.length > 0 ? s.charAt(0).toUpperCase() + s.slice(1) : s;
    }

    // "14:02 · 3 hours ago", with the date when it wasn't today. Spelled
    // out rather than with %n plurals, which need a translation loaded.
    function since(ms) {
        if (isNaN(ms)) {
            return "";
        }
        const then = new Date(ms);
        const now = new Date();
        const today = then.toDateString() === now.toDateString();
        const time = then.toLocaleTimeString(Qt.locale(), Locale.ShortFormat);
        const stamp = today ? time : qsTr("%1, %2").arg(then.toLocaleDateString(Qt.locale(), "d MMM")).arg(time);
        const minutes = Math.floor((now - then) / 60000);
        let ago;
        if (minutes < 1) {
            ago = qsTr("moments ago");
        } else if (minutes < 60) {
            ago = minutes === 1 ? qsTr("1 minute ago") : qsTr("%1 minutes ago").arg(minutes);
        } else if (minutes < 48 * 60) {
            const h = Math.floor(minutes / 60);
            ago = h === 1 ? qsTr("1 hour ago") : qsTr("%1 hours ago").arg(h);
        } else {
            ago = qsTr("%1 days ago").arg(Math.floor(minutes / 1440));
        }
        return qsTr("%1 · %2").arg(stamp).arg(ago);
    }

    // Processor time as "2 h 05 min 09 s".
    function cpuTime(s) {
        if (isNaN(s)) {
            return "";
        }
        s = Math.round(s);
        const h = Math.floor(s / 3600);
        const m = Math.floor(s / 60) % 60;
        const sec = s % 60;
        const two = n => (n < 10 ? "0" : "") + n;
        if (h > 0) {
            return qsTr("%1 h %2 min %3 s").arg(h).arg(two(m)).arg(two(sec));
        }
        if (m > 0) {
            return qsTr("%1 min %2 s").arg(m).arg(two(sec));
        }
        return qsTr("%1 s").arg(sec);
    }

    // What a nice value means, not just the number.
    function priority(n) {
        if (n < 0) {
            return qsTr("Raised (%1)").arg(n);
        }
        return n === 0 ? qsTr("Normal") : qsTr("Lowered (+%1)").arg(n);
    }

    function size(v) {
        return isNaN(v) ? "" : Format.scaled(v, "");
    }

    function percent(v) {
        return Number(v).toLocaleString(Qt.locale(), "f", 1) + "%";
    }

    function percentOfCore(v) {
        return qsTr("%1 of a core").arg(dialog.percent(v));
    }

    // Each new thing shown starts at its top.
    Connections {
        target: dialog.details
        function onShown() {
            scroll.QQC2.ScrollBar.vertical.position = 0;
        }
        // Into a member; Back keeps the list where it was left.
        function onCanGoBackChanged() {
            if (dialog.details.canGoBack) {
                scroll.QQC2.ScrollBar.vertical.position = 0;
            }
        }
    }

    parent: QQC2.Overlay.overlay
    anchors.centerIn: parent
    modal: true
    focus: true
    closePolicy: QQC2.Popup.CloseOnEscape | QQC2.Popup.CloseOnPressOutside
    width: Math.min(parent ? parent.width - Kirigami.Units.gridUnit * 2 : 0, Kirigami.Units.gridUnit * 30)
    height: Math.min(implicitHeight, parent ? parent.height - Kirigami.Units.gridUnit * 3 : implicitHeight)
    padding: Math.round(Kirigami.Units.gridUnit * 1.3)

    enter: Transition {
        NumberAnimation {
            property: "opacity"
            from: 0
            to: 1
            duration: Kirigami.Units.shortDuration
        }
    }
    exit: Transition {
        NumberAnimation {
            property: "opacity"
            from: 1
            to: 0
            duration: Kirigami.Units.shortDuration
        }
    }

    QQC2.Overlay.modal: Rectangle {
        color: Qt.rgba(0, 0, 0, 0.35)
    }

    background: Rectangle {
        radius: 14
        color: Kirigami.Theme.backgroundColor
        border.width: 1
        border.color: Qt.alpha(Kirigami.Theme.textColor, 0.16)
    }

    // A label small above its value, which is what anyone is here for; the
    // value can be selected, so a path or a command line can be copied. An
    // empty value leaves the row out.
    component Property: ColumnLayout {
        id: property
        property string label
        property string value
        Layout.fillWidth: true
        Layout.leftMargin: Kirigami.Units.largeSpacing
        Layout.rightMargin: Kirigami.Units.largeSpacing
        Layout.topMargin: Kirigami.Units.smallSpacing
        Layout.bottomMargin: Kirigami.Units.smallSpacing
        visible: value.length > 0
        spacing: 0
        Accessible.role: Accessible.StaticText
        Accessible.name: label + ", " + value

        QQC2.Label {
            text: property.label
            font: Kirigami.Theme.smallFont
            opacity: 0.65
            textFormat: Text.PlainText
        }
        TextEdit {
            Layout.fillWidth: true
            text: property.value
            readOnly: true
            selectByMouse: true
            wrapMode: TextEdit.WrapAnywhere
            textFormat: TextEdit.PlainText
            color: Kirigami.Theme.textColor
            selectionColor: Kirigami.Theme.highlightColor
            selectedTextColor: Kirigami.Theme.highlightedTextColor
            font: Kirigami.Theme.defaultFont
            Accessible.ignored: true
        }
    }

    contentItem: ColumnLayout {
        Accessible.role: Accessible.Dialog
        Accessible.name: dialog.details.title
        spacing: Kirigami.Units.largeSpacing

        RowLayout {
            Layout.fillWidth: true
            spacing: Kirigami.Units.smallSpacing

            QQC2.ToolButton {
                visible: dialog.details.canGoBack
                icon.name: LayoutMirroring.enabled ? "go-next" : "go-previous"
                display: QQC2.AbstractButton.IconOnly
                text: qsTr("Back")
                QQC2.ToolTip.text: text
                QQC2.ToolTip.visible: hovered
                QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
                onClicked: dialog.details.back()
            }
            QQC2.Label {
                Layout.fillWidth: true
                text: dialog.details.title
                font.bold: true
                font.pointSize: Kirigami.Theme.defaultFont.pointSize * 1.15
                elide: Text.ElideRight
                textFormat: Text.PlainText
                Accessible.role: Accessible.Heading
            }
            QQC2.ToolButton {
                icon.name: "window-close"
                display: QQC2.AbstractButton.IconOnly
                text: qsTr("Close")
                QQC2.ToolTip.text: text
                QQC2.ToolTip.visible: hovered
                QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
                onClicked: dialog.close()
            }
        }

        QQC2.ScrollView {
            id: scroll
            Layout.fillWidth: true
            Layout.fillHeight: true
            implicitHeight: body.implicitHeight
            contentWidth: availableWidth
            // The bar beside the cards, not over their edge.
            rightPadding: QQC2.ScrollBar.vertical.visible ? QQC2.ScrollBar.vertical.width + Kirigami.Units.smallSpacing : 0
            QQC2.ScrollBar.horizontal.policy: QQC2.ScrollBar.AlwaysOff

            ColumnLayout {
                id: body
                width: scroll.availableWidth
                spacing: Kirigami.Units.gridUnit

                QQC2.BusyIndicator {
                    Layout.alignment: Qt.AlignHCenter
                    visible: dialog.details.loading
                    running: visible
                }
                QQC2.Label {
                    Layout.fillWidth: true
                    visible: dialog.details.gone && !dialog.details.loading
                    text: qsTr("Process %1 has exited.").arg(dialog.details.pid)
                    wrapMode: Text.Wrap
                    opacity: 0.8
                }

                // One process.
                ColumnLayout {
                    Layout.fillWidth: true
                    visible: !dialog.details.group && !dialog.details.loading && !dialog.details.gone
                    spacing: Kirigami.Units.gridUnit

                    Section {
                        title: qsTr("Process")
                        Property {
                            label: qsTr("Name")
                            value: dialog.details.name
                        }
                        Property {
                            label: qsTr("Process ID")
                            value: String(dialog.details.pid)
                        }
                        Property {
                            label: qsTr("Started By")
                            value: dialog.details.parent <= 0 ? "" : dialog.details.parentName.length > 0 ? qsTr("%1 (%2)").arg(dialog.details.parentName).arg(dialog.details.parent) : String(dialog.details.parent)
                        }
                        Property {
                            label: qsTr("User")
                            value: dialog.details.user
                        }
                        Property {
                            label: qsTr("Running Since")
                            value: dialog.since(dialog.details.started)
                        }
                        Property {
                            label: qsTr("State")
                            value: dialog.capitalise(dialog.details.state)
                        }
                        Property {
                            label: qsTr("Threads")
                            value: dialog.details.threads > 0 ? String(dialog.details.threads) : ""
                        }
                        Property {
                            label: qsTr("Priority")
                            value: dialog.priority(dialog.details.nice)
                        }
                    }
                    Section {
                        title: qsTr("Program")
                        Property {
                            label: qsTr("Executable")
                            value: dialog.details.executable
                        }
                        Property {
                            label: qsTr("Command Line")
                            value: dialog.details.commandLine
                        }
                        Property {
                            label: qsTr("Application")
                            value: dialog.details.application
                        }
                        Property {
                            label: qsTr("Unit")
                            value: dialog.details.unit
                        }
                    }
                    Section {
                        title: qsTr("Resources")
                        Property {
                            label: qsTr("Processor Time")
                            value: dialog.cpuTime(dialog.details.cpuTime)
                        }
                        Property {
                            label: qsTr("Resident Memory")
                            value: dialog.size(dialog.details.memory)
                        }
                        Property {
                            label: qsTr("Proportional Memory")
                            value: dialog.size(dialog.details.pss)
                        }
                        Property {
                            label: qsTr("Private Memory")
                            value: dialog.size(dialog.details.privateMemory)
                        }
                        Property {
                            label: qsTr("Swapped Out")
                            value: dialog.size(dialog.details.swap)
                        }
                        Property {
                            label: qsTr("Open Files")
                            value: dialog.details.openFiles >= 0 ? String(dialog.details.openFiles) : ""
                        }
                    }
                }

                // An application of several processes.
                ColumnLayout {
                    Layout.fillWidth: true
                    visible: dialog.details.group
                    spacing: Kirigami.Units.gridUnit

                    Section {
                        title: qsTr("Application")
                        Property {
                            label: qsTr("Name")
                            value: dialog.details.title
                        }
                        Property {
                            label: qsTr("Application ID")
                            value: dialog.details.appId
                        }
                        Property {
                            label: qsTr("Processes")
                            value: String(dialog.details.processes)
                        }
                        Property {
                            label: qsTr("Processor")
                            value: dialog.percentOfCore(dialog.details.cpu)
                        }
                        Property {
                            label: qsTr("Resident Memory")
                            value: dialog.size(dialog.details.groupMemory)
                        }
                        Property {
                            label: dialog.details.units.indexOf("\n") >= 0 ? qsTr("Units") : qsTr("Unit")
                            value: dialog.details.units
                        }
                    }
                    Section {
                        title: qsTr("Processes")
                        footer: dialog.details.processes > dialog.details.memberNames.length ? qsTr("The %1 busiest of %2.").arg(dialog.details.memberNames.length).arg(dialog.details.processes) : ""
                        Repeater {
                            model: dialog.details.memberNames
                            SectionRow {
                                required property int index
                                required property string modelData
                                Layout.fillWidth: true
                                title: modelData
                                subtitle: qsTr("PID %1 · %2 · %3").arg(dialog.details.memberPids[index] ?? "").arg(dialog.percent(dialog.details.memberCpu[index] ?? 0)).arg(Format.scaled(dialog.details.memberMemory[index] ?? 0, ""))
                                chevron: true
                                onClicked: dialog.details.openMember(index)
                            }
                        }
                    }
                }
            }
        }
    }
}
