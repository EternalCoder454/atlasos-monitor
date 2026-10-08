pragma Singleton

import QtQml

// How the pages write their figures. A figure the machine doesn't report is
// NaN (or 0 for a size) and shows as a dash, never as a real-looking 0.
QtObject {
    readonly property string dash: "–"
    // Qt.locale() builds a new object each call; the tables format every
    // cell each second.
    readonly property var locale: Qt.locale()
    // The units, translated once: a qsTr call looks the string up each
    // time, and the pages format their figures every second.
    readonly property string ghzFormat: qsTr("%1 GHz")
    readonly property string mhzFormat: qsTr("%1 MHz")
    readonly property string celsiusFormat: qsTr("%1 °C")
    readonly property string wattsFormat: qsTr("%1 W")
    readonly property string hoursMinutesFormat: qsTr("%1h %2m")
    readonly property string minutesFormat: qsTr("%1m")
    readonly property string whFormat: qsTr("%1 Wh")
    readonly property string shareFormat: qsTr("%1 of %2")

    function percent(v) {
        return isNaN(v) ? dash : Math.round(v) + "%";
    }

    // 3600 MHz as "3.60 GHz", under a gigahertz as "800 MHz".
    function mhz(v) {
        if (isNaN(v) || v <= 0) {
            return dash;
        }
        if (v >= 1000) {
            return ghzFormat.arg(Number(v / 1000).toLocaleString(locale, "f", 2));
        }
        return mhzFormat.arg(Math.round(v));
    }

    function celsius(v) {
        return isNaN(v) ? dash : celsiusFormat.arg(Math.round(v));
    }

    // Binary units as the Go version writes them: "10.71 GiB", "512.3 MiB",
    // "48 KiB".
    function scaled(v, suffix) {
        const steps = [[1099511627776, "TiB", 2], [1073741824, "GiB", 2], [1048576, "MiB", 1], [1024, "KiB", 0]];
        for (const [unit, name, places] of steps) {
            if (v >= unit) {
                return Number(v / unit).toLocaleString(locale, "f", places) + " " + name + suffix;
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

    // A YYYY-MM-DD date as the locale writes a short one, the year in full
    // ("5/5/2025", "05.05.2025"). Anything else is returned as it came.
    function date(iso) {
        const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(iso);
        if (m === null) {
            return iso;
        }
        const d = new Date(2000, 0, 1);
        d.setFullYear(Number(m[1]), Number(m[2]) - 1, Number(m[3]));
        // 2025-02-31 rolls over into March: not a date.
        if (d.getMonth() !== Number(m[2]) - 1) {
            return iso;
        }
        return d.toLocaleDateString(locale, locale.dateFormat(Locale.ShortFormat).replace(/y+/, "yyyy"));
    }

    // Bytes per second.
    function rate(v) {
        return isNaN(v) || v < 0 ? dash : scaled(v, "/s");
    }

    // Watts, a decimal under 10 W as the Go version: "4.6 W", "88 W". A
    // device that doesn't say reads NaN or 0.
    function watts(v) {
        if (isNaN(v) || v <= 0) {
            return dash;
        }
        // Decimals by the rounded figure, so 9.96 is "10 W", not "10.0 W".
        return wattsFormat.arg(Number(v).toLocaleString(locale, "f", Math.round(v * 10) / 10 < 10 ? 1 : 0));
    }

    // Seconds as the Go version writes them: "2h 15m", "40m"; -1 is a dash.
    function duration(s) {
        if (s <= 0) {
            return dash;
        }
        const h = Math.floor(s / 3600);
        const m = Math.floor(s / 60) % 60;
        return h > 0 ? hoursMinutesFormat.arg(h).arg(m) : minutesFormat.arg(m);
    }

    // Watt-hours, one decimal.
    function wh(v) {
        return isNaN(v) || v <= 0 ? dash : whFormat.arg(Number(v).toLocaleString(locale, "f", 1));
    }

    // A count the device may not report (-1).
    function count(v) {
        return v < 0 ? dash : Number(v).toLocaleString(locale, "f", 0);
    }

    // "x of y", for a part of a whole.
    function share(part, whole) {
        return whole > 0 ? shareFormat.arg(size(part)).arg(scaled(whole, "")) : dash;
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
        return qsTr("%1 years").arg(Number(h / (24 * 365)).toLocaleString(locale, "f", 1));
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
