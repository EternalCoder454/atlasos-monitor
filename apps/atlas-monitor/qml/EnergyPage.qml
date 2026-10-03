pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

// The apps keeping the processor busy, each with Ease Off (or Put Back),
// and the switch that eases them off automatically. Easing an app off
// lowers its systemd CPU weight: it keeps running and keeps its work, and
// just stops winning against everything else.
AtlasPage {
    id: page

    required property var energy

    readonly property bool usable: page.energy.unavailable.length === 0

    function unavailableText(key) {
        switch (key) {
        case "noAppUnits":
            return qsTr("Apps aren't started in systemd units here, so there is nothing to ease off.");
        case "noCpuController":
            return qsTr("This system doesn't let an app's share of the processor change without administrator rights.");
        case "noManager":
            return qsTr("There is no systemd user manager in this session.");
        case "noPipeWire":
            return qsTr("Energy Saver needs pw-dump (from pipewire-utils) to tell which apps are playing sound, and it isn't installed.");
        }
        return "";
    }

    // One decimal: a busy app reads "104.5% of a core".
    function ofACore(v) {
        return qsTr("%1% of a core").arg(Number(v).toLocaleString(Qt.locale(), "f", 1));
    }

    function statusText(key, cpu) {
        const share = page.ofACore(cpu);
        switch (key) {
        case "easedAuto":
            return qsTr("Eased off automatically · %1").arg(share);
        case "easedManual":
            return qsTr("Eased off by you · %1").arg(share);
        case "keptSound":
            return qsTr("Playing or recording sound, so left alone · %1").arg(share);
        case "keptTerminal":
            return qsTr("A terminal, so left alone · %1").arg(share);
        case "keptNever":
            return qsTr("Set never to ease off · %1").arg(share);
        case "keptByUser":
            return qsTr("Put back by you, so left alone · %1").arg(share);
        case "keptInUse":
            return qsTr("In use, so left alone · %1").arg(share);
        case "keptOther":
            return qsTr("Its priority was set by something else, so left alone · %1").arg(share);
        case "busy":
            return page.energy.automatic ? qsTr("Busy · %1 · eased off if it keeps this up").arg(share) : qsTr("Busy · %1").arg(share);
        }
        return qsTr("Using %1").arg(share);
    }

    function failure(reason, name, detail) {
        switch (reason) {
        case "noAnswer":
            return qsTr("Couldn't change %1: the session's service manager didn't answer.").arg(name);
        case "gone":
            return qsTr("%1 has closed.").arg(name);
        case "notAnApp":
            return qsTr("%1 isn't an app Energy Saver can ease off.").arg(name);
        case "refused":
            return detail.length > 0 ? qsTr("The service manager wouldn't change %1: %2").arg(name).arg(detail) : qsTr("The service manager wouldn't change %1.").arg(name);
        }
        return qsTr("Couldn't change %1.").arg(name);
    }

    title: qsTr("Energy Saver")

    QQC2.Label {
        Layout.fillWidth: true
        wrapMode: Text.Wrap
        opacity: 0.7
        text: qsTr("Easing an app off puts it behind everything else on the processor. It keeps running and keeps its work; it just stops winning. This lasts until the app is restarted, and can be undone.")
    }

    Kirigami.InlineMessage {
        Layout.fillWidth: true
        type: Kirigami.MessageType.Warning
        visible: page.energy.loaded && !page.usable
        text: page.unavailableText(page.energy.unavailable)
    }

    Kirigami.InlineMessage {
        id: notice
        Layout.fillWidth: true
        type: Kirigami.MessageType.Warning
        showCloseButton: true

        Timer {
            id: hide
            interval: 10000
            onTriggered: notice.visible = false
        }
    }

    Connections {
        target: page.energy

        function onFailed(name, reason, detail) {
            notice.text = page.failure(reason, name, detail);
            notice.visible = true;
            hide.restart();
        }
    }

    Section {
        SectionRow {
            title: qsTr("Ease Off Busy Apps Automatically")
            subtitle: qsTr("An app that keeps a core busy for half a minute is put behind the rest, and put back when it calms down. Apps playing or recording sound, the app you're using, and terminals are left alone.")
            showSwitch: true
            switchChecked: page.energy.automatic
            enabled: page.usable
            onSwitchToggled: checked => page.energy.setAutomaticEasing(checked)
        }
    }

    QQC2.Label {
        visible: !page.energy.loaded
        opacity: 0.7
        text: qsTr("Measuring what apps are using…")
    }

    QQC2.Label {
        visible: page.energy.loaded && page.usable && page.energy.count === 0
        Layout.fillWidth: true
        wrapMode: Text.Wrap
        opacity: 0.7
        text: qsTr("No app is busy right now.")
    }

    Section {
        title: qsTr("Apps")
        visible: page.energy.count > 0

        Repeater {
            model: page.energy

            SectionRow {
                id: row

                required property string appId
                required property string name
                required property string icon
                required property real cpu
                required property string status
                required property bool never

                readonly property bool eased: status === "easedAuto" || status === "easedManual"

                iconName: row.icon
                title: row.name
                subtitle: page.statusText(row.status, row.cpu)

                SecondaryButton {
                    text: row.eased ? qsTr("Put Back") : qsTr("Ease Off")
                    // One at a time: the manager answers them in order, and
                    // the row shows the outcome before the next.
                    enabled: !page.energy.busy
                    onClicked: row.eased ? page.energy.restore(row.appId) : page.energy.ease(row.appId)
                    Accessible.description: row.name
                }
                QQC2.ToolButton {
                    icon.name: "overflow-menu"
                    display: QQC2.AbstractButton.IconOnly
                    text: qsTr("More Options for %1").arg(row.name)
                    QQC2.ToolTip.text: qsTr("More Options")
                    QQC2.ToolTip.visible: hovered && !options.visible
                    QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
                    // Its right edge under the button's, so it stays over the card.
                    onClicked: options.popup(this, mirrored ? 0 : width - options.width, height + 4)

                    ContextMenu {
                        id: options

                        ContextMenuItem {
                            text: qsTr("Never Ease Off Automatically")
                            // ContextMenuItem draws no check box of its own.
                            icon.name: row.never ? "checkmark" : ""
                            Accessible.checkable: true
                            Accessible.checked: row.never
                            onTriggered: page.energy.setNever(row.appId, !row.never)
                        }
                    }
                }
            }
        }
    }
}
