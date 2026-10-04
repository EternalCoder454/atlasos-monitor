import QtQuick
import QtQuick.Layouts

// A sidebar entry that folds out into sub-entries: Disk, then its drives.
// Atlas.Ui's SidebarGroup with NavButton's look. Put NavButtons with
// `sub: true` inside. While folded the header shows as selected if one of
// its entries is, so the sidebar still says where you are.
//
// Compact (icons only) there is no room for sub-entries: the group is its
// header's icon alone, and a click on it emits `activated` for the app to
// open the group's first entry.
ColumnLayout {
    id: root

    property alias text: header.text
    property string iconName
    property alias compact: header.compact
    property bool expanded: true
    default property alias items: entries.data
    // True when one of the entries is selected.
    readonly property bool holdsSelection: {
        const c = entries.children;
        for (let i = 0; i < c.length; ++i) {
            if (c[i].selected === true) {
                return true;
            }
        }
        return false;
    }

    signal activated

    Layout.fillWidth: true
    // The gap between entries; match the sidebar column it sits in.
    spacing: 2

    NavButton {
        id: header
        Layout.fillWidth: true
        icon.name: root.iconName
        disclosure: true
        expanded: root.expanded
        selected: (!root.expanded || root.compact) && root.holdsSelection
        onClicked: {
            if (root.compact) {
                root.activated();
                return;
            }
            root.expanded = !root.expanded;
        }
    }

    ColumnLayout {
        id: entries
        Layout.fillWidth: true
        visible: root.expanded && !root.compact
        spacing: root.spacing
    }
}
