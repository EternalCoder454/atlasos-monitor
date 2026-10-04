import QtQuick
import Atlas.Ui

// An icon-only button outside a toolbar: Atlas.Ui's ToolbarButton, which
// never takes the focus (it is made to leave it with an editor), here
// reachable with Tab and ringed while it has it. `text` is its tooltip and
// its accessible name.
ToolbarButton {
    id: control

    focusPolicy: Qt.StrongFocus
    // Space it handles itself.
    Keys.onPressed: event => {
        if ((event.key === Qt.Key_Return || event.key === Qt.Key_Enter) && !event.isAutoRepeat) {
            event.accepted = true;
            control.click();
        }
    }

    // ToolbarButton's background radius.
    AtlasFocusRing {
        radius: 6
        shown: control.visualFocus
    }
}
