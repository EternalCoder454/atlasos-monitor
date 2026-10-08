import QtQuick
import org.kde.kirigami as Kirigami

// A theme icon (Kirigami.Icon) that stays under what covers it. With Quick's
// software renderer Kirigami.Icon draws as a render node, and whenever a
// repaint touches a part of it the whole icon is painted again over the
// dialogs, popups and menus in front of it. As a layer it is an ordinary image
// node, which the renderer orders and clips right. With OpenGL and the other
// backends it is the plain icon, with no layer. (Telamon.Ui 2.0.5 has
// TelamonIcon for this; the app moves to it when it moves to that release.)
Kirigami.Icon {
    layer.enabled: GraphicsInfo.api === GraphicsInfo.Software
}
