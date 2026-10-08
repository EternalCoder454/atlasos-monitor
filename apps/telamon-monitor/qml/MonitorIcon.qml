import QtQuick
import org.kde.kirigami as Kirigami

// A theme icon (Kirigami.Icon) that stays under what covers it. With Quick's
// software renderer Kirigami.Icon draws as a render node, and whenever a
// repaint touches a part of it the whole icon is painted again over the
// dialogs, popups and menus in front of it. As a layer it is an ordinary image
// node, which the renderer orders and clips right. With OpenGL and the other
// backends it is the plain icon, with no layer.
//
// The layer is live only for a moment after the icon changes (its source,
// colour, size, state or theme; see _refresh()): a live layer on the software
// renderer is drawn again on every frame and keeps the app busy (an idle
// window used 8% of a core instead of 0.3%). Same as Telamon.Ui's TelamonIcon.
Kirigami.Icon {
    id: icon

    layer.enabled: GraphicsInfo.api === GraphicsInfo.Software
    layer.live: false

    // Draws the layer again: live for a few frames, so the icon's own changes
    // (several in a row, an image that arrives later) are in it.
    function _refresh(): void {
        if (layer.enabled) {
            layer.live = true;
            refreshTimer.restart();
        }
    }

    onSourceChanged: _refresh()
    onColorChanged: _refresh()
    onWidthChanged: _refresh()
    onHeightChanged: _refresh()
    onIsMaskChanged: _refresh()
    onActiveChanged: _refresh()
    onSelectedChanged: _refresh()
    onEnabledChanged: _refresh()
    onVisibleChanged: _refresh()
    onValidChanged: _refresh()
    onStatusChanged: _refresh()
    onFallbackChanged: _refresh()
    onPlaceholderChanged: _refresh()
    onPaintedWidthChanged: _refresh()
    onPaintedHeightChanged: _refresh()
    onWindowChanged: _refresh()
    Kirigami.Theme.onColorsChanged: _refresh()
    Component.onCompleted: _refresh()

    Timer {
        id: refreshTimer
        interval: 100
        onTriggered: icon.layer.live = false
    }
}
