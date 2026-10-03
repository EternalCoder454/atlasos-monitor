pragma Singleton

import QtQml

// How the pages write their figures. A figure the machine doesn't report is
// NaN (or 0 for a size) and shows as a dash, never as a real-looking 0.
QtObject {
    readonly property string dash: "–"

    function percent(v) {
        return isNaN(v) ? dash : Math.round(v) + "%";
    }

    // 3600 MHz as "3.60 GHz", under a gigahertz as "800 MHz".
    function mhz(v) {
        if (isNaN(v) || v <= 0) {
            return dash;
        }
        if (v >= 1000) {
            return qsTr("%1 GHz").arg(Number(v / 1000).toLocaleString(Qt.locale(), "f", 2));
        }
        return qsTr("%1 MHz").arg(Math.round(v));
    }

    function celsius(v) {
        return isNaN(v) ? dash : qsTr("%1 °C").arg(Math.round(v));
    }

    // A size the kernel may not report (a cache): 0 is a dash.
    function bytes(v) {
        return v > 0 ? Qt.locale().formattedDataSize(v) : dash;
    }

    // A size that is really 0 sometimes (swap in use).
    function size(v) {
        return Qt.locale().formattedDataSize(Math.max(0, v));
    }

    // "x of y", for a part of a whole.
    function share(part, whole) {
        return whole > 0 ? qsTr("%1 of %2").arg(Qt.locale().formattedDataSize(part)).arg(Qt.locale().formattedDataSize(whole)) : dash;
    }

    // The span a 60-sample chart covers at this refresh interval (ms).
    function span(interval) {
        // Spelled out: qsTr's %n plurals need a translation loaded, and
        // without one show "minute(s)".
        const s = Math.round(60 * interval / 1000);
        if (s === 60) {
            return qsTr("1 minute");
        }
        return s % 60 === 0 ? qsTr("%1 minutes").arg(s / 60) : qsTr("%1 seconds").arg(s);
    }
}
