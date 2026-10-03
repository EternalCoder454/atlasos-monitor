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

    // Binary units as the Go version writes them: "10.71 GiB", "512.3 MiB",
    // "48 KiB".
    function scaled(v, suffix) {
        const steps = [[1099511627776, "TiB", 2], [1073741824, "GiB", 2], [1048576, "MiB", 1], [1024, "KiB", 0]];
        for (const [unit, name, places] of steps) {
            if (v >= unit) {
                return Number(v / unit).toLocaleString(Qt.locale(), "f", places) + " " + name + suffix;
            }
        }
        return Math.round(Math.max(0, v)) + " B" + suffix;
    }

    // A size the kernel may not report (a cache): 0 or -1 is a dash.
    function bytes(v) {
        return v > 0 ? scaled(v, "") : dash;
    }

    // A size that is really 0 sometimes (swap in use); -1 is a dash.
    function size(v) {
        return v < 0 ? dash : scaled(v, "");
    }

    // Bytes per second.
    function rate(v) {
        return isNaN(v) || v < 0 ? dash : scaled(v, "/s");
    }

    // A count the device may not report (-1).
    function count(v) {
        return v < 0 ? dash : Number(v).toLocaleString(Qt.locale(), "f", 0);
    }

    // "x of y", for a part of a whole.
    function share(part, whole) {
        return whole > 0 ? qsTr("%1 of %2").arg(size(part)).arg(scaled(whole, "")) : dash;
    }

    // How long a drive has been powered on.
    function hours(h) {
        if (h < 0) {
            return dash;
        }
        if (h < 48) {
            return qsTr("%1 hours").arg(h);
        }
        if (h < 24 * 365) {
            return qsTr("%1 days").arg(Math.floor(h / 24));
        }
        return qsTr("%1 years").arg(Number(h / (24 * 365)).toLocaleString(Qt.locale(), "f", 1));
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
