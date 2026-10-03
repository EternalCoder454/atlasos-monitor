pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

// What starts when you log in, each with a switch. The desktop's own
// background pieces are behind "Show System Entries".
AtlasPage {
    id: page

    required property var startup

    property bool showSystem: false
    readonly property var s: page.startup
    // How many rows show with the toggle as it is.
    readonly property int shown: {
        let n = 0;
        for (let i = 0; i < page.s.ids.length; ++i) {
            if (page.showSystem || !page.s.plumbing[i]) {
                ++n;
            }
        }
        return n;
    }

    function note(key) {
        switch (key) {
        case "notThisDesktop":
            return qsTr("Not used by this desktop");
        case "missingProgram":
            return qsTr("Its program isn't installed");
        case "noCommand":
            return qsTr("Has nothing to start");
        case "turnedOff":
            return qsTr("Turned off in its own settings");
        case "byUnit":
            return qsTr("Started by the desktop itself");
        }
        return "";
    }

    function lock(key) {
        switch (key) {
        case "required":
            return qsTr("Part of AtlasOS");
        case "session":
            return qsTr("Needed to start your session");
        }
        return "";
    }

    function failure(key, name) {
        switch (key) {
        case "locked":
            return qsTr("%1 can't be switched off.").arg(name);
        case "notFound":
            return qsTr("%1 is no longer installed.").arg(name);
        case "notEnableable":
            return qsTr("%1 can't be set to start at login.").arg(name);
        case "noAnswer":
            return qsTr("Couldn't switch %1: the session's service manager didn't answer.").arg(name);
        }
        return qsTr("Couldn't switch %1.").arg(name);
    }

    title: qsTr("Startup")

    RowLayout {
        Layout.fillWidth: true
        spacing: Kirigami.Units.largeSpacing

        QQC2.Label {
            Layout.fillWidth: true
            wrapMode: Text.Wrap
            opacity: 0.7
            text: qsTr("Programs that start when you log in. A change takes effect at your next login.")
        }
        QQC2.Label {
            text: qsTr("Show System Entries")
        }
        AtlasSwitch {
            checked: page.showSystem
            onToggled: page.showSystem = checked
            Accessible.name: qsTr("Show System Entries")
        }
        SecondaryButton {
            text: qsTr("Refresh")
            icon.name: "view-refresh"
            enabled: !page.s.loading
            onClicked: page.s.refresh()
        }
    }

    Kirigami.InlineMessage {
        Layout.fillWidth: true
        type: Kirigami.MessageType.Warning
        visible: page.s.error.length > 0
        text: page.failure(page.s.error, page.s.errorName)
    }

    QQC2.Label {
        visible: page.s.loaded && page.shown === 0
        Layout.fillWidth: true
        wrapMode: Text.Wrap
        opacity: 0.7
        text: page.s.ids.length > 0 ? qsTr("Nothing of your own starts at login. The desktop's own background pieces are under Show System Entries.") : qsTr("Nothing starts at login.")
    }

    Section {
        visible: page.shown > 0
        footer: page.s.loaded && !page.s.units ? qsTr("The session's service manager didn't answer, so services that start at login aren't listed.") : ""

        Repeater {
            model: page.s.ids.length

            SectionRow {
                required property int index
                readonly property string note: page.note(page.s.notes[index] ?? "")
                readonly property string lock: page.lock(page.s.locks[index] ?? "")
                readonly property string what: page.s.subtitles[index] ?? ""

                visible: page.showSystem || !(page.s.plumbing[index] ?? false)
                iconName: page.s.icons[index] || "application-x-executable"
                title: page.s.names[index] ?? ""
                subtitle: [note, lock, what].filter(t => t.length > 0).join(" · ")
                value: page.s.states[index] === "running" ? qsTr("Running") : page.s.states[index] === "failed" ? qsTr("Failed") : ""
                showSwitch: true
                switchChecked: page.s.enabled[index] ?? false
                // While a change is on its way the model ignores another,
                // and the switch springs back to what it says.
                enabled: page.s.switchable[index] ?? false
                onSwitchToggled: checked => page.s.setEnabled(page.s.ids[index], checked)
            }
        }
    }

    Component.onCompleted: page.s.refresh()
}
